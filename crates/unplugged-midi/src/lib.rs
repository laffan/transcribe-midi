//! External MIDI input.
//!
//! Port enumeration and connection are platform work; deciding what an arriving event
//! *means* is not, and lives in `unplugged_core::recorder`. This crate is kept as thin
//! as the audio backend for the same reason — it cannot be exercised on a machine
//! without a MIDI interface.
//!
//! **Timestamps come from the caller, not from `midir`.** Per DECISIONS.md Finding 1,
//! midir does not populate `message.timestamp` on iOS. Rather than have two timing
//! paths, incoming events are stamped by the host against the audio clock at the moment
//! the callback fires, on every platform.

use std::fmt;
use std::sync::{Arc, Mutex};

/// A parsed channel-voice message. Anything else on the wire is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiEvent {
    NoteOn { pitch: u8, velocity: u8, channel: u8 },
    NoteOff { pitch: u8, channel: u8 },
    ControlChange { controller: u8, value: u8, channel: u8 },
    PitchBend { value: i16, channel: u8 },
}

impl MidiEvent {
    pub fn channel(&self) -> u8 {
        match self {
            MidiEvent::NoteOn { channel, .. }
            | MidiEvent::NoteOff { channel, .. }
            | MidiEvent::ControlChange { channel, .. }
            | MidiEvent::PitchBend { channel, .. } => *channel,
        }
    }

    /// Parse one MIDI message.
    ///
    /// Returns `None` for system messages, running status continuations and truncated
    /// packets — all of which are normal traffic, not errors.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let status = *bytes.first()?;
        if status < 0x80 {
            return None; // running status: no status byte in this packet
        }

        let channel = status & 0x0F;
        match status & 0xF0 {
            0x80 => Some(MidiEvent::NoteOff {
                pitch: *bytes.get(1)? & 0x7F,
                channel,
            }),
            0x90 => {
                let pitch = *bytes.get(1)? & 0x7F;
                let velocity = *bytes.get(2)? & 0x7F;
                // Note-on with velocity 0 is a note-off. Most hardware releases this way.
                Some(if velocity == 0 {
                    MidiEvent::NoteOff { pitch, channel }
                } else {
                    MidiEvent::NoteOn { pitch, velocity, channel }
                })
            }
            0xB0 => Some(MidiEvent::ControlChange {
                controller: *bytes.get(1)? & 0x7F,
                value: *bytes.get(2)? & 0x7F,
                channel,
            }),
            0xE0 => {
                let lsb = *bytes.get(1)? as i16 & 0x7F;
                let msb = *bytes.get(2)? as i16 & 0x7F;
                Some(MidiEvent::PitchBend {
                    value: ((msb << 7) | lsb) - 8192,
                    channel,
                })
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MidiPort {
    /// Stable within a session; ports are addressed by this.
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiError(pub String);

impl fmt::Display for MidiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for MidiError {}

pub type MidiResult<T> = Result<T, MidiError>;

/// Called for every accepted event, on midir's callback thread.
pub type EventSink = Arc<dyn Fn(MidiEvent) + Send + Sync>;

/// Shared configuration, read from the callback thread.
#[derive(Debug, Default)]
struct Filter {
    /// `None` accepts every channel ("omni"), which is the sane default for a single
    /// controller — most send on channel 1 but not all, and silence is a baffling
    /// first-run experience.
    channel: Option<u8>,
}

// ---------------------------------------------------------------------------
// Apple
// ---------------------------------------------------------------------------

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple {
    use super::*;
    use midir::{MidiInput, MidiInputConnection};

    const CLIENT_NAME: &str = "Unplugged";

    pub struct MidiInputHost {
        connection: Mutex<Option<MidiInputConnection<()>>>,
        filter: Arc<Mutex<Filter>>,
        connected: Mutex<Option<MidiPort>>,
    }

    impl MidiInputHost {
        pub fn new() -> Self {
            MidiInputHost {
                connection: Mutex::new(None),
                filter: Arc::new(Mutex::new(Filter::default())),
                connected: Mutex::new(None),
            }
        }

        pub fn ports(&self) -> MidiResult<Vec<MidiPort>> {
            let input = MidiInput::new(CLIENT_NAME)
                .map_err(|e| MidiError(format!("could not open a MIDI client: {e}")))?;

            Ok(input
                .ports()
                .iter()
                .enumerate()
                .map(|(index, port)| MidiPort {
                    // CoreMIDI's own id is not exposed by midir, and names are not
                    // unique (two identical controllers). Index plus name is stable for
                    // as long as the port list is, which is all a session needs.
                    id: format!("{index}:{}", input.port_name(port).unwrap_or_default()),
                    name: input.port_name(port).unwrap_or_else(|_| format!("Port {index}")),
                })
                .collect())
        }

        pub fn connected(&self) -> Option<MidiPort> {
            self.connected.lock().ok().and_then(|c| c.clone())
        }

        pub fn set_channel_filter(&self, channel: Option<u8>) {
            if let Ok(mut filter) = self.filter.lock() {
                filter.channel = channel.map(|c| c & 0x0F);
            }
        }

        pub fn disconnect(&self) {
            if let Ok(mut slot) = self.connection.lock() {
                if let Some(connection) = slot.take() {
                    connection.close();
                }
            }
            if let Ok(mut connected) = self.connected.lock() {
                *connected = None;
            }
        }

        pub fn connect(&self, port_id: &str, sink: EventSink) -> MidiResult<MidiPort> {
            // Only one input at a time; reconnecting replaces the previous port.
            self.disconnect();

            let input = MidiInput::new(CLIENT_NAME)
                .map_err(|e| MidiError(format!("could not open a MIDI client: {e}")))?;

            let ports = input.ports();
            let matched = ports
                .iter()
                .enumerate()
                .find(|(index, port)| {
                    let name = input.port_name(port).unwrap_or_default();
                    format!("{index}:{name}") == port_id
                })
                .map(|(_, port)| port.clone())
                .ok_or_else(|| MidiError(format!("no MIDI input port matching '{port_id}'")))?;

            let name = input.port_name(&matched).unwrap_or_else(|_| port_id.to_string());
            let filter = Arc::clone(&self.filter);

            let connection = input
                .connect(
                    &matched,
                    "unplugged-in",
                    move |_timestamp, bytes, _| {
                        // ---- midir callback thread ----
                        // `_timestamp` is deliberately unused: it is unpopulated on iOS,
                        // so the host stamps events against the audio clock instead.
                        let Some(event) = MidiEvent::parse(bytes) else {
                            return;
                        };
                        if let Ok(filter) = filter.lock() {
                            if let Some(only) = filter.channel {
                                if event.channel() != only {
                                    return;
                                }
                            }
                        }
                        sink(event);
                    },
                    (),
                )
                .map_err(|e| MidiError(format!("could not connect to '{name}': {e}")))?;

            let port = MidiPort { id: port_id.to_string(), name };

            if let Ok(mut slot) = self.connection.lock() {
                *slot = Some(connection);
            }
            if let Ok(mut connected) = self.connected.lock() {
                *connected = Some(port.clone());
            }
            Ok(port)
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub use apple::MidiInputHost;

// ---------------------------------------------------------------------------
// Null backend
// ---------------------------------------------------------------------------

/// Enumerates nothing and receives nothing, so the app runs off-Apple for UI work.
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
pub struct MidiInputHost {
    filter: Mutex<Filter>,
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
impl MidiInputHost {
    pub fn new() -> Self {
        MidiInputHost {
            filter: Mutex::new(Filter::default()),
        }
    }

    pub fn ports(&self) -> MidiResult<Vec<MidiPort>> {
        Ok(Vec::new())
    }

    pub fn connected(&self) -> Option<MidiPort> {
        None
    }

    pub fn set_channel_filter(&self, channel: Option<u8>) {
        if let Ok(mut filter) = self.filter.lock() {
            filter.channel = channel.map(|c| c & 0x0F);
        }
    }

    pub fn disconnect(&self) {}

    pub fn connect(&self, port_id: &str, _sink: EventSink) -> MidiResult<MidiPort> {
        Err(MidiError(format!(
            "MIDI input is not available on this platform (asked for '{port_id}')"
        )))
    }
}

impl Default for MidiInputHost {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_on_and_off_parse() {
        assert_eq!(
            MidiEvent::parse(&[0x90, 60, 100]),
            Some(MidiEvent::NoteOn { pitch: 60, velocity: 100, channel: 0 })
        );
        assert_eq!(
            MidiEvent::parse(&[0x85, 64, 0]),
            Some(MidiEvent::NoteOff { pitch: 64, channel: 5 })
        );
    }

    #[test]
    fn note_on_with_zero_velocity_parses_as_note_off() {
        // The dominant way hardware sends releases; treating it as a note-on would leave
        // every note hanging.
        assert_eq!(
            MidiEvent::parse(&[0x90, 60, 0]),
            Some(MidiEvent::NoteOff { pitch: 60, channel: 0 })
        );
    }

    #[test]
    fn the_channel_nibble_is_decoded() {
        for channel in 0u8..16 {
            let event = MidiEvent::parse(&[0x90 | channel, 60, 100]).unwrap();
            assert_eq!(event.channel(), channel);
        }
    }

    #[test]
    fn pitch_bend_is_centred_on_zero() {
        // 0x2000 (8192) is centre.
        assert_eq!(
            MidiEvent::parse(&[0xE0, 0x00, 0x40]),
            Some(MidiEvent::PitchBend { value: 0, channel: 0 })
        );
        // Minimum and maximum.
        assert_eq!(
            MidiEvent::parse(&[0xE0, 0x00, 0x00]),
            Some(MidiEvent::PitchBend { value: -8192, channel: 0 })
        );
        assert_eq!(
            MidiEvent::parse(&[0xE0, 0x7F, 0x7F]),
            Some(MidiEvent::PitchBend { value: 8191, channel: 0 })
        );
    }

    #[test]
    fn control_change_parses() {
        assert_eq!(
            MidiEvent::parse(&[0xB2, 64, 127]),
            Some(MidiEvent::ControlChange { controller: 64, value: 127, channel: 2 })
        );
    }

    #[test]
    fn truncated_and_system_messages_are_ignored_rather_than_panicking() {
        // These all arrive in normal traffic; none may index out of bounds.
        assert_eq!(MidiEvent::parse(&[]), None);
        assert_eq!(MidiEvent::parse(&[0x90]), None, "missing data bytes");
        assert_eq!(MidiEvent::parse(&[0x90, 60]), None, "missing velocity");
        assert_eq!(MidiEvent::parse(&[0xF8]), None, "timing clock");
        assert_eq!(MidiEvent::parse(&[0xF0, 0x7E, 0xF7]), None, "sysex");
        assert_eq!(MidiEvent::parse(&[0x40, 0x50]), None, "running status continuation");
    }

    #[test]
    fn the_null_backend_enumerates_nothing_and_refuses_to_connect() {
        let host = MidiInputHost::new();
        if cfg!(not(any(target_os = "macos", target_os = "ios"))) {
            assert!(host.ports().unwrap().is_empty());
            assert!(host.connect("0:whatever", Arc::new(|_| {})).is_err());
            assert!(host.connected().is_none());
            host.disconnect(); // must not panic
        }
    }
}
