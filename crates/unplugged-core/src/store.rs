//! Project persistence: the directory layout, atomic writes, and CRUD.
//!
//! ```text
//! <root>/<project-id>/
//!   project.json
//!   tracks/<track-id>.mid
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{CoreError, Result};
use crate::model::*;
use crate::smf;

const MANIFEST_FILE: &str = "project.json";
const TRACKS_DIR: &str = "tracks";

/// Owns the projects directory. Cheap to construct; holds no state beyond the path.
#[derive(Debug, Clone)]
pub struct ProjectStore {
    root: PathBuf,
}

impl ProjectStore {
    /// `root` is the directory that *contains* project directories. Created on demand.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        ProjectStore { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn project_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    fn manifest_path(&self, id: &str) -> PathBuf {
        self.project_dir(id).join(MANIFEST_FILE)
    }

    fn ensure_root(&self) -> Result<()> {
        fs::create_dir_all(&self.root).map_err(|e| CoreError::io(&self.root, e))
    }

    // -----------------------------------------------------------------------
    // Listing
    // -----------------------------------------------------------------------

    /// All projects, most recently modified first.
    ///
    /// A directory whose manifest is missing or unreadable is skipped rather than
    /// failing the whole listing — one corrupt project should not make the picker
    /// unusable. The bad directory is reported separately by [`Self::list_with_errors`].
    pub fn list(&self) -> Result<Vec<ProjectSummary>> {
        Ok(self.list_with_errors()?.projects)
    }

    /// Like [`Self::list`], but also reports each directory that could not be read, so
    /// the UI console can surface them instead of them silently vanishing.
    pub fn list_with_errors(&self) -> Result<ProjectListing> {
        self.ensure_root()?;

        let mut summaries = Vec::new();
        let mut errors = Vec::new();

        let entries = fs::read_dir(&self.root).map_err(|e| CoreError::io(&self.root, e))?;
        for entry in entries {
            let entry = entry.map_err(|e| CoreError::io(&self.root, e))?;
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            match self.read_manifest(&id) {
                Ok(manifest) => summaries.push(ProjectSummary::from(&manifest)),
                Err(e) => errors.push(ProjectListError {
                    id,
                    message: e.to_string(),
                }),
            }
        }

        summaries.sort_by(|a, b| b.modified_at_ms.cmp(&a.modified_at_ms).then(a.name.cmp(&b.name)));
        Ok(ProjectListing {
            projects: summaries,
            errors,
        })
    }

    pub fn exists(&self, id: &str) -> bool {
        self.manifest_path(id).is_file()
    }

    // -----------------------------------------------------------------------
    // Create
    // -----------------------------------------------------------------------

    /// Create a new project directory with one empty track.
    ///
    /// The id is slugified from `name` and deduplicated, so creating three projects
    /// called "Untitled" yields `untitled`, `untitled-2`, `untitled-3`.
    pub fn create(&self, name: &str, tempo_bpm: f64, time_signature: TimeSignature) -> Result<ProjectManifest> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CoreError::EmptyName);
        }

        self.ensure_root()?;
        let id = self.unique_id(&slugify(name));
        let now = now_ms();

        let manifest = ProjectManifest {
            schema_version: SCHEMA_VERSION,
            id: id.clone(),
            name: name.to_string(),
            tempo_bpm,
            time_signature,
            ppq: DEFAULT_PPQ,
            tracks: vec![TrackMeta::for_index(0)],
            created_at_ms: now,
            modified_at_ms: now,
        };
        manifest.validate()?;

        let dir = self.project_dir(&id);
        if dir.exists() {
            return Err(CoreError::ProjectExists(id));
        }
        fs::create_dir_all(dir.join(TRACKS_DIR)).map_err(|e| CoreError::io(&dir, e))?;

        // Write the track's (empty) SMF first, then the manifest. If we crash between
        // the two, the orphaned .mid is harmless; the reverse order would leave a
        // manifest pointing at a file that does not exist.
        for track in &manifest.tracks {
            self.write_track_notes(&id, track, &[], manifest.ppq)?;
        }
        self.write_manifest(&manifest)?;

        Ok(manifest)
    }

    // -----------------------------------------------------------------------
    // Read
    // -----------------------------------------------------------------------

    pub fn read_manifest(&self, id: &str) -> Result<ProjectManifest> {
        let path = self.manifest_path(id);
        if !path.is_file() {
            return Err(CoreError::ProjectNotFound(id.to_string()));
        }
        let raw = fs::read_to_string(&path).map_err(|e| CoreError::io(&path, e))?;
        let manifest: ProjectManifest =
            serde_json::from_str(&raw).map_err(|source| CoreError::Json { path: path.clone(), source })?;

        if manifest.schema_version > SCHEMA_VERSION {
            return Err(CoreError::SchemaTooNew(
                id.to_string(),
                manifest.schema_version,
                SCHEMA_VERSION,
            ));
        }
        manifest.validate()?;
        Ok(manifest)
    }

    /// Load the manifest and every track's notes.
    pub fn load(&self, id: &str) -> Result<Project> {
        let manifest = self.read_manifest(id)?;
        let mut tracks = Vec::with_capacity(manifest.tracks.len());

        for meta in &manifest.tracks {
            let notes = self.read_track_notes(id, meta)?;
            tracks.push(Track {
                meta: meta.clone(),
                notes,
                ppq: manifest.ppq,
            });
        }

        Ok(Project { manifest, tracks })
    }

    fn read_track_notes(&self, project_id: &str, meta: &TrackMeta) -> Result<Vec<Note>> {
        let path = self.project_dir(project_id).join(meta.relative_midi_path());
        // A missing track file means an empty track, not a broken project. This is the
        // state right after a crash between manifest write and track write.
        if !path.is_file() {
            return Ok(Vec::new());
        }
        let bytes = fs::read(&path).map_err(|e| CoreError::io(&path, e))?;
        let (notes, _) = smf::smf_bytes_to_notes(&bytes, &path)?;
        Ok(notes)
    }

    // -----------------------------------------------------------------------
    // Write
    // -----------------------------------------------------------------------

    /// Persist a whole project. Bumps `modified_at_ms`.
    pub fn save(&self, project: &mut Project) -> Result<()> {
        project.manifest.validate()?;

        // Manifest track list and loaded tracks must agree, or we would write notes to
        // a track the manifest does not list (and lose them on next load).
        if project.manifest.tracks.len() != project.tracks.len() {
            return Err(CoreError::TrackNotFound(
                "manifest track list does not match loaded tracks".into(),
            ));
        }

        let dir = self.project_dir(&project.manifest.id);
        fs::create_dir_all(dir.join(TRACKS_DIR)).map_err(|e| CoreError::io(&dir, e))?;

        for track in &project.tracks {
            project.manifest.track(&track.meta.id)?;
            self.write_track_notes(&project.manifest.id, &track.meta, &track.notes, project.manifest.ppq)?;
        }

        project.manifest.modified_at_ms = now_ms();
        self.write_manifest(&project.manifest)
    }

    fn write_track_notes(&self, project_id: &str, meta: &TrackMeta, notes: &[Note], ppq: u16) -> Result<()> {
        let bytes = smf::notes_to_smf_bytes(notes, ppq, &meta.name)?;
        let path = self.project_dir(project_id).join(meta.relative_midi_path());
        write_atomic(&path, &bytes)
    }

    fn write_manifest(&self, manifest: &ProjectManifest) -> Result<()> {
        let json = serde_json::to_vec_pretty(manifest).map_err(|source| CoreError::Json {
            path: self.manifest_path(&manifest.id),
            source,
        })?;
        write_atomic(&self.manifest_path(&manifest.id), &json)
    }

    // -----------------------------------------------------------------------
    // Rename / delete
    // -----------------------------------------------------------------------

    /// Change the display name. The id and directory are deliberately left alone, so
    /// nothing breaks if this is interrupted and no open handles go stale.
    pub fn rename(&self, id: &str, new_name: &str) -> Result<ProjectManifest> {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            return Err(CoreError::EmptyName);
        }
        if new_name.chars().count() > MAX_NAME_LEN {
            return Err(CoreError::NameTooLong(new_name.chars().count()));
        }

        let mut manifest = self.read_manifest(id)?;
        manifest.name = new_name.to_string();
        manifest.modified_at_ms = now_ms();
        manifest.validate()?;
        self.write_manifest(&manifest)?;
        Ok(manifest)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        // Guard against `id` escaping the projects root via `..` or an absolute path.
        // The frontend supplies this string, so it is untrusted input.
        if !is_safe_id(id) {
            return Err(CoreError::ProjectNotFound(id.to_string()));
        }
        let dir = self.project_dir(id);
        if !dir.is_dir() {
            return Err(CoreError::ProjectNotFound(id.to_string()));
        }
        fs::remove_dir_all(&dir).map_err(|e| CoreError::io(&dir, e))
    }

    // -----------------------------------------------------------------------
    // Track-level operations
    // -----------------------------------------------------------------------

    pub fn add_track(&self, project_id: &str, name: Option<&str>) -> Result<TrackMeta> {
        let mut manifest = self.read_manifest(project_id)?;
        let mut meta = TrackMeta::for_index(manifest.tracks.len());
        // Keep ids unique even after tracks have been deleted from the middle.
        while manifest.tracks.iter().any(|t| t.id == meta.id) {
            meta.id = format!("{}-{}", meta.id, manifest.tracks.len() + 1);
        }
        if let Some(name) = name {
            let name = name.trim();
            if !name.is_empty() {
                meta.name = name.to_string();
            }
        }

        self.write_track_notes(project_id, &meta, &[], manifest.ppq)?;
        manifest.tracks.push(meta.clone());
        manifest.modified_at_ms = now_ms();
        self.write_manifest(&manifest)?;
        Ok(meta)
    }

    pub fn delete_track(&self, project_id: &str, track_id: &str) -> Result<()> {
        let mut manifest = self.read_manifest(project_id)?;
        let Some(index) = manifest.tracks.iter().position(|t| t.id == track_id) else {
            return Err(CoreError::TrackNotFound(track_id.to_string()));
        };
        let removed = manifest.tracks.remove(index);

        manifest.modified_at_ms = now_ms();
        // Manifest first: once the track is delisted, the stale .mid is unreachable.
        self.write_manifest(&manifest)?;

        let path = self.project_dir(project_id).join(removed.relative_midi_path());
        if path.is_file() {
            fs::remove_file(&path).map_err(|e| CoreError::io(&path, e))?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn unique_id(&self, base: &str) -> String {
        if !self.project_dir(base).exists() {
            return base.to_string();
        }
        for n in 2..10_000 {
            let candidate = format!("{base}-{n}");
            if !self.project_dir(&candidate).exists() {
                return candidate;
            }
        }
        // Vanishingly unlikely; fall back to a timestamp rather than looping forever.
        format!("{base}-{}", now_ms())
    }
}

/// Write via a temp file in the same directory, then rename.
///
/// `rename` within a directory is atomic on both APFS and every filesystem we care
/// about, so a crash or a full disk mid-write leaves the previous file intact instead of
/// a half-written `project.json` that will not parse.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|e| CoreError::io(parent, e))?;

    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("tmp");
    let tmp = parent.join(format!(".{file_name}.tmp"));

    fs::write(&tmp, bytes).map_err(|e| CoreError::io(&tmp, e))?;
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp); // Best effort; do not mask the real error.
            Err(CoreError::io(path, e))
        }
    }
}

/// Reject anything that could traverse outside the projects root.
fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && !id.contains('/')
        && !id.contains('\\')
        && !id.contains('\0')
}

/// Lowercase, ASCII alphanumerics and dashes only.
///
/// The id becomes a directory name on a case-insensitive filesystem (APFS default), so
/// it is kept deliberately narrow. Non-ASCII names are fine as *display* names; they
/// simply do not contribute to the id, which is why the empty-result fallback exists.
pub fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_dash = true; // leading dashes suppressed

    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }

    // Names that are entirely non-ASCII (e.g. "プロジェクト") slugify to nothing.
    if out.is_empty() {
        out.push_str("project");
    }
    out.truncate(64);
    while out.ends_with('-') {
        out.pop();
    }
    out
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn store() -> (TempDir, ProjectStore) {
        let dir = TempDir::new().unwrap();
        let store = ProjectStore::new(dir.path().join("projects"));
        (dir, store)
    }

    fn ts() -> TimeSignature {
        TimeSignature::default()
    }

    #[test]
    fn create_then_load_round_trips() {
        let (_g, store) = store();
        let manifest = store.create("My Song", 128.0, TimeSignature::new(3, 4).unwrap()).unwrap();

        assert_eq!(manifest.id, "my-song");
        assert_eq!(manifest.tracks.len(), 1, "a new project starts with one track");

        let project = store.load(&manifest.id).unwrap();
        assert_eq!(project.manifest.name, "My Song");
        assert_eq!(project.manifest.tempo_bpm, 128.0);
        assert_eq!(project.manifest.time_signature, TimeSignature::new(3, 4).unwrap());
        assert_eq!(project.tracks.len(), 1);
        assert!(project.tracks[0].notes.is_empty());
        assert_eq!(project.tracks[0].ppq, manifest.ppq, "track PPQ is stamped from the manifest");
    }

    #[test]
    fn notes_survive_a_save_and_reload() {
        let (_g, store) = store();
        let manifest = store.create("Notes", DEFAULT_TEMPO, ts()).unwrap();
        let mut project = store.load(&manifest.id).unwrap();

        project.tracks[0].insert_note(Note::new(60, 0, 480, 100, 0).unwrap()).unwrap();
        project.tracks[0].insert_note(Note::new(64, 480, 480, 90, 0).unwrap()).unwrap();
        store.save(&mut project).unwrap();

        let reloaded = store.load(&manifest.id).unwrap();
        assert_eq!(reloaded.tracks[0].notes, project.tracks[0].notes);
    }

    #[test]
    fn duplicate_names_get_distinct_ids_and_do_not_overwrite() {
        let (_g, store) = store();
        let a = store.create("Untitled", DEFAULT_TEMPO, ts()).unwrap();
        let b = store.create("Untitled", DEFAULT_TEMPO, ts()).unwrap();
        let c = store.create("Untitled", DEFAULT_TEMPO, ts()).unwrap();

        assert_eq!((a.id.as_str(), b.id.as_str(), c.id.as_str()), ("untitled", "untitled-2", "untitled-3"));
        assert_eq!(store.list().unwrap().len(), 3, "all three must coexist");
    }

    #[test]
    fn rename_changes_display_name_but_keeps_id_and_notes() {
        let (_g, store) = store();
        let manifest = store.create("Old Name", DEFAULT_TEMPO, ts()).unwrap();

        let mut project = store.load(&manifest.id).unwrap();
        project.tracks[0].insert_note(Note::new(72, 0, 240, 100, 0).unwrap()).unwrap();
        store.save(&mut project).unwrap();

        let renamed = store.rename(&manifest.id, "New Name").unwrap();
        assert_eq!(renamed.name, "New Name");
        assert_eq!(renamed.id, manifest.id, "id must be stable across rename");

        let reloaded = store.load(&manifest.id).unwrap();
        assert_eq!(reloaded.tracks[0].notes.len(), 1, "rename must not touch note data");
    }

    #[test]
    fn delete_removes_the_project_and_its_files() {
        let (_g, store) = store();
        let manifest = store.create("Doomed", DEFAULT_TEMPO, ts()).unwrap();
        assert!(store.exists(&manifest.id));

        store.delete(&manifest.id).unwrap();

        assert!(!store.exists(&manifest.id));
        assert!(!store.project_dir(&manifest.id).exists());
        assert!(store.list().unwrap().is_empty());
        assert!(matches!(store.load(&manifest.id), Err(CoreError::ProjectNotFound(_))));
    }

    #[test]
    fn delete_refuses_path_traversal_ids() {
        let (guard, store) = store();
        let victim = guard.path().join("victim.txt");
        fs::write(&victim, b"do not delete me").unwrap();

        for evil in ["..", "../..", "../victim.txt", "/etc", "a/../.."] {
            assert!(store.delete(evil).is_err(), "'{evil}' must be rejected");
        }
        assert!(victim.exists(), "a sibling of the projects root must survive");
    }

    #[test]
    fn list_is_ordered_most_recently_modified_first() {
        let (_g, store) = store();
        let a = store.create("Alpha", DEFAULT_TEMPO, ts()).unwrap();
        let b = store.create("Beta", DEFAULT_TEMPO, ts()).unwrap();

        // Force a distinct, unambiguous modification time rather than sleeping.
        let mut manifest = store.read_manifest(&a.id).unwrap();
        manifest.modified_at_ms = b.modified_at_ms + 10_000;
        store.write_manifest(&manifest).unwrap();

        let listed = store.list().unwrap();
        assert_eq!(listed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec![a.id.as_str(), b.id.as_str()]);
    }

    #[test]
    fn one_corrupt_project_does_not_break_the_listing() {
        let (_g, store) = store();
        let good = store.create("Good", DEFAULT_TEMPO, ts()).unwrap();
        let bad = store.create("Bad", DEFAULT_TEMPO, ts()).unwrap();

        fs::write(store.project_dir(&bad.id).join(MANIFEST_FILE), b"{ not json").unwrap();

        let listing = store.list_with_errors().unwrap();
        assert_eq!(listing.projects.len(), 1, "the readable project must still be listed");
        assert_eq!(listing.projects[0].id, good.id);
        assert_eq!(listing.errors.len(), 1, "the unreadable one must be reported");
        assert_eq!(listing.errors[0].id, bad.id);
    }

    #[test]
    fn a_manifest_from_the_future_is_refused_rather_than_guessed_at() {
        let (_g, store) = store();
        let manifest = store.create("Future", DEFAULT_TEMPO, ts()).unwrap();

        let path = store.project_dir(&manifest.id).join(MANIFEST_FILE);
        let raw = fs::read_to_string(&path).unwrap();
        let bumped = raw.replace(
            &format!("\"schema_version\": {SCHEMA_VERSION}"),
            &format!("\"schema_version\": {}", SCHEMA_VERSION + 1),
        );
        fs::write(&path, bumped).unwrap();

        assert!(matches!(store.load(&manifest.id), Err(CoreError::SchemaTooNew(..))));
    }

    #[test]
    fn add_and_delete_track_keep_manifest_and_files_in_step() {
        let (_g, store) = store();
        let manifest = store.create("Tracks", DEFAULT_TEMPO, ts()).unwrap();

        let added = store.add_track(&manifest.id, Some("Bass")).unwrap();
        assert_eq!(added.name, "Bass");

        let loaded = store.load(&manifest.id).unwrap();
        assert_eq!(loaded.tracks.len(), 2);
        assert!(store.project_dir(&manifest.id).join(added.relative_midi_path()).is_file());

        store.delete_track(&manifest.id, &added.id).unwrap();

        let after = store.load(&manifest.id).unwrap();
        assert_eq!(after.tracks.len(), 1);
        assert!(!store.project_dir(&manifest.id).join(added.relative_midi_path()).exists());
        assert!(store.delete_track(&manifest.id, "no-such-track").is_err());
    }

    #[test]
    fn a_missing_track_file_loads_as_an_empty_track() {
        // This is the on-disk state after a crash between manifest and track writes.
        let (_g, store) = store();
        let manifest = store.create("Partial", DEFAULT_TEMPO, ts()).unwrap();
        let path = store.project_dir(&manifest.id).join(manifest.tracks[0].relative_midi_path());
        fs::remove_file(&path).unwrap();

        let project = store.load(&manifest.id).unwrap();
        assert_eq!(project.tracks.len(), 1);
        assert!(project.tracks[0].notes.is_empty());
    }

    #[test]
    fn atomic_write_leaves_no_temp_files_behind() {
        let (_g, store) = store();
        let manifest = store.create("Clean", DEFAULT_TEMPO, ts()).unwrap();
        let mut project = store.load(&manifest.id).unwrap();
        store.save(&mut project).unwrap();

        let leftovers: Vec<_> = fs::read_dir(store.project_dir(&manifest.id))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "found temp files: {leftovers:?}");
    }

    #[test]
    fn save_refuses_a_project_whose_tracks_disagree_with_its_manifest() {
        let (_g, store) = store();
        let manifest = store.create("Mismatch", DEFAULT_TEMPO, ts()).unwrap();
        let mut project = store.load(&manifest.id).unwrap();

        // A track present in memory but absent from the manifest would be written and
        // then silently lost on the next load.
        project.tracks.push(Track::new(TrackMeta::for_index(5), project.manifest.ppq));
        assert!(store.save(&mut project).is_err());
    }

    #[test]
    fn invalid_project_parameters_are_rejected_at_creation() {
        let (_g, store) = store();
        assert!(store.create("", DEFAULT_TEMPO, ts()).is_err(), "empty name");
        assert!(store.create("   ", DEFAULT_TEMPO, ts()).is_err(), "whitespace-only name");
        assert!(store.create("Fast", 9000.0, ts()).is_err(), "tempo above range");
        assert!(store.create("Slow", 1.0, ts()).is_err(), "tempo below range");
        assert!(store.create("NaN", f64::NAN, ts()).is_err(), "non-finite tempo");
    }

    #[test]
    fn names_are_trimmed_not_rejected_when_padded() {
        let (_g, store) = store();
        let manifest = store.create("  Padded  ", DEFAULT_TEMPO, ts()).unwrap();
        assert_eq!(manifest.name, "Padded");
        assert_eq!(manifest.id, "padded");
    }

    #[test]
    fn slugify_handles_the_awkward_cases() {
        assert_eq!(slugify("My Song"), "my-song");
        assert_eq!(slugify("  Hello   World  "), "hello-world");
        assert_eq!(slugify("!!!"), "project", "punctuation-only falls back");
        assert_eq!(slugify("プロジェクト"), "project", "non-ASCII falls back");
        assert_eq!(slugify("Über Cool"), "ber-cool");
        assert_eq!(slugify("a/../../etc/passwd"), "a-etc-passwd", "separators cannot survive");
        assert!(!slugify(&"x".repeat(500)).is_empty());
        assert!(slugify(&"x".repeat(500)).len() <= 64);
    }

    #[test]
    fn a_non_ascii_name_still_produces_a_usable_project() {
        let (_g, store) = store();
        let manifest = store.create("プロジェクト", DEFAULT_TEMPO, ts()).unwrap();
        assert_eq!(manifest.name, "プロジェクト", "display name is preserved verbatim");
        assert_eq!(manifest.id, "project");
        assert_eq!(store.load(&manifest.id).unwrap().manifest.name, "プロジェクト");
    }
}
