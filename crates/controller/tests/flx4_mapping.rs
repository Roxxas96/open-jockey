//! The draft DDJ-FLX4 mapping and its capture checklist load the way the app loads them
//! (`--controller <path>` and `midi capture --checklist <path>`), and bind the core
//! controls of both decks.

use std::path::PathBuf;

use actions::{ActionKind, Target};
use controller::builtin::load_mapping;
use controller::capture::parse_checklist;
use controller::mapping::{InputBinding, Mapping, MidiSource};
use engine::DeckId;

fn flx4_file(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../controllers/ddj-flx4/{name}"))
}

fn mapping() -> Mapping {
    load_mapping(flx4_file("mapping.toml").to_str().unwrap()).unwrap()
}

fn bound(mapping: &Mapping, midi: MidiSource) -> Vec<&InputBinding> {
    mapping.inputs.iter().filter(|b| b.midi == midi).collect()
}

fn has(mapping: &Mapping, midi: MidiSource, kind: ActionKind, target: Target) -> bool {
    bound(mapping, midi)
        .iter()
        .any(|b| b.kind == kind && b.target == target)
}

#[test]
fn flx4_mapping_loads_with_its_port_and_jog_resolution() {
    let mapping = mapping();
    assert_eq!(mapping.controller.port, "DDJ-FLX4");
    assert!(mapping.controller.jog_steps_per_turn.is_some());
}

#[test]
fn flx4_binds_play_cue_and_channel_fader_of_both_decks() {
    let mapping = mapping();
    // Two decks, no layering: deck N sends on MIDI channel N (0-based N - 1 here).
    for deck in 1..=2u8 {
        let channel = deck - 1;
        let target = Target::Deck(DeckId::new(usize::from(deck) - 1).unwrap());
        let note = |note| MidiSource::Note { channel, note };
        assert!(
            has(&mapping, note(0x0B), ActionKind::PlayPause, target),
            "deck {deck} PLAY"
        );
        assert!(
            has(&mapping, note(0x0C), ActionKind::Cue, target),
            "deck {deck} CUE"
        );
        let fader = MidiSource::Cc14 {
            channel,
            msb: 0x13,
            lsb: 0x33,
        };
        assert!(
            has(&mapping, fader, ActionKind::Fader, target),
            "deck {deck} CH FADER"
        );
    }
}

#[test]
fn flx4_binds_the_crossfader() {
    let mapping = mapping();
    let crossfader = MidiSource::Cc14 {
        channel: 6,
        msb: 0x1F,
        lsb: 0x3F,
    };
    assert!(has(
        &mapping,
        crossfader,
        ActionKind::Crossfader,
        Target::Global
    ));
}

#[test]
fn flx4_checklist_parses() {
    let text = std::fs::read_to_string(flx4_file("checklist.toml")).unwrap();
    let checklist = parse_checklist(&text).unwrap();
    assert_eq!(checklist.controller.as_deref(), Some("Pioneer DDJ-FLX4"));
    for deck in 1..=2 {
        for step in ["play", "cue", "fader"] {
            let id = format!("deck{deck}.{step}");
            assert!(
                checklist.steps.iter().any(|s| s.id == id),
                "missing step {id}"
            );
        }
    }
    assert!(checklist.steps.iter().any(|s| s.id == "mixer.crossfader"));
}

#[test]
fn flx4_binds_only_decks_one_and_two() {
    let mapping = mapping();
    for binding in &mapping.inputs {
        if let Target::Deck(deck) = binding.target {
            assert!(
                deck == DeckId::new(0).unwrap() || deck == DeckId::new(1).unwrap(),
                "binding {:?} targets a deck the FLX4 does not have",
                binding.midi
            );
        }
    }
}
