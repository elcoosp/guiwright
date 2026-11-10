//! Key sequences with modifiers **and explicit `KeyUp`** (§4.3 gap: freya's
//! `press_key` "sends `KeyDown` only — we drive raw `PlatformEvent::Keyboard`
//! with modifiers+KeyUp").
//!
//! Playwright semantics: `press("Control+a")` = modifier(s) down → key
//! down → key up → modifier(s) up. The sequence is total: every key that
//! goes down comes back up, in reverse order.

/// Keyboard event phase (raw `PlatformEvent::Keyboard` vocabulary).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyPhase {
    Down,
    Up,
}

/// One keyboard event of a press sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub phase: KeyPhase,
    /// Normalized key name (lowercase single char, or named key like
    /// "enter", "tab", "escape").
    pub key: String,
    /// Modifiers held during this event (canonical §8.4 names, sorted).
    pub modifiers: Vec<String>,
}

/// Normalize one key token (§9 keyboard vocabulary).
pub fn normalize_key(key: &str) -> String {
    key.trim().to_ascii_lowercase()
}

/// Canonical §8.4 modifier names: `shift` / `ctrl` / `alt` / `meta`.
/// Long aliases (`Control`, `Cmd`, `Super`, `Option`, `Windows`,
/// `Command`) fold onto the short form so every adapter event carries
/// the same token regardless of how the caller spelled it (the driver
/// and kit pass the user's spelling through verbatim; the adapter owns
/// the canonical form the platform sees).
fn canonical_modifier(lowercased: &str) -> &str {
    match lowercased {
        "control" => "ctrl",
        "cmd" | "command" | "super" | "windows" => "meta",
        "option" => "alt",
        other => other,
    }
}

/// Normalize the modifier list (sorted, deduped, canonical §8.4 names).
pub fn normalize_modifiers(modifiers: &[String]) -> Vec<String> {
    let mut out: Vec<String> = modifiers
        .iter()
        .map(|m| canonical_modifier(m.trim().to_ascii_lowercase().as_str()).to_string())
        .filter(|m| !m.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The full press sequence for `key` with `modifiers` (§4.3).
///
/// 1. modifier down events (in sorted order),
/// 2. key down (carrying the modifiers),
/// 3. key up (carrying the modifiers),
/// 4. modifier up events (reverse sorted order).
pub fn key_sequence(key: &str, modifiers: &[String]) -> Vec<KeyEvent> {
    let mods = normalize_modifiers(modifiers);
    let key = normalize_key(key);
    let mut seq = Vec::with_capacity(2 + mods.len() * 2);
    for m in &mods {
        seq.push(KeyEvent {
            phase: KeyPhase::Down,
            key: m.clone(),
            modifiers: mods.clone(),
        });
    }
    seq.push(KeyEvent {
        phase: KeyPhase::Down,
        key: key.clone(),
        modifiers: mods.clone(),
    });
    seq.push(KeyEvent {
        phase: KeyPhase::Up,
        key: key.clone(),
        modifiers: mods.clone(),
    });
    for m in mods.iter().rev() {
        seq.push(KeyEvent {
            phase: KeyPhase::Up,
            key: m.clone(),
            modifiers: mods.clone(),
        });
    }
    seq
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_key_down_then_up() {
        let seq = key_sequence("Enter", &[]);
        assert_eq!(seq.len(), 2);
        assert_eq!(seq[0].phase, KeyPhase::Down);
        assert_eq!(seq[0].key, "enter");
        assert_eq!(seq[0].modifiers.len(), 0);
        assert_eq!(seq[1].phase, KeyPhase::Up);
        assert_eq!(seq[1].key, "enter");
    }

    #[test]
    fn modifiers_go_down_first_and_come_up_last_in_reverse() {
        let seq = key_sequence("a", &["Shift".into(), "Control".into()]);
        assert_eq!(seq.len(), 6);
        assert_eq!(
            (seq[0].phase, seq[0].key.as_str()),
            (KeyPhase::Down, "ctrl")
        );
        assert_eq!(
            (seq[1].phase, seq[1].key.as_str()),
            (KeyPhase::Down, "shift")
        );
        assert_eq!(
            seq[2],
            KeyEvent {
                phase: KeyPhase::Down,
                key: "a".into(),
                modifiers: vec!["ctrl".into(), "shift".into()],
            }
        );
        assert_eq!(seq[3].phase, KeyPhase::Up);
        assert_eq!(seq[3].key, "a");
        assert_eq!(seq[4].key, "shift", "modifiers release in reverse");
        assert_eq!(seq[5].key, "ctrl");
        // Every down has a matching up.
        for ev in &seq {
            assert!(
                seq.iter()
                    .any(|other| other.phase != ev.phase && other.key == ev.key)
            );
        }
    }

    #[test]
    fn single_chars_normalize_lowercase() {
        let seq = key_sequence("A", &[]);
        assert_eq!(seq[0].key, "a");
        let seq = key_sequence(" ESCAPE ", &[]);
        assert_eq!(seq[0].key, "escape");
    }

    #[test]
    fn modifiers_are_deduped_and_sorted() {
        let seq = key_sequence("c", &["ctrl".into(), " CTRL ".into(), String::new()]);
        assert_eq!(seq.len(), 4, "one modifier pair + key pair");
        assert_eq!(seq[0].key, "ctrl");
    }
}
