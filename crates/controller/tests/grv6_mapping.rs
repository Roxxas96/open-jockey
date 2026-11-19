//! The draft DDJ-GRV6 mapping and its capture checklist load the way the app loads them
//! (`--controller <path>` and `midi capture --checklist <path>`), and bind the core
//! controls of every deck.

use std::path::PathBuf;

use actions::{ActionKind, Target};
use controller::builtin::load_mapping;
use controller::capture::parse_checklist;
use controller::mapping::{InputBinding, Mapping, MidiSource};
use engine::DeckId;

fn grv6_file(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../controllers/ddj-grv6/{name}"))
}

fn mapping() -> Mapping {
    load_mapping(grv6_file("mapping.toml").to_str().unwrap()).unwrap()
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
fn grv6_mapping_loads_with_its_port_and_jog_resolution() {
    let mapping = mapping();
    assert_eq!(mapping.controller.port, "DDJ-GRV6");
    assert!(mapping.controller.jog_steps_per_turn.is_some());
}

#[test]
fn grv6_binds_play_cue_and_channel_fader_of_every_deck() {
    let mapping = mapping();
    // Hardware layering: deck N sends on MIDI channel N (0-based N - 1 here).
    for deck in 1..=4u8 {
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
fn grv6_binds_the_crossfader() {
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
fn grv6_checklist_parses() {
    let text = std::fs::read_to_string(grv6_file("checklist.toml")).unwrap();
    let checklist = parse_checklist(&text).unwrap();
    assert_eq!(checklist.controller.as_deref(), Some("Pioneer DDJ-GRV6"));
    for deck in 1..=4 {
        for step in ["play", "cue", "fader", "deck_select"] {
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
fn grv6_deck_buttons_move_the_side_with_the_hardware() {
    use actions::{ControlState, Input, Side, apply};
    use controller::MidiMessage;
    use controller::mapping::Resolver;
    use engine::{EngineConfig, engine};

    let mapping = mapping();
    let mut resolver = Resolver::new(&mapping);
    let mut state = ControlState::default();
    let (mut controller, _renderer) = engine(EngineConfig {
        sample_rate: 44_100,
        max_block_frames: 64,
    });
    let mut send = |msg: MidiMessage, state: &mut ControlState| {
        for (action, input) in resolver.resolve(&msg, state) {
            apply(&action, input, 0, state, &mut controller);
        }
    };
    let on = |channel, note| MidiMessage::NoteOn {
        channel,
        note,
        velocity: 127,
    };
    let off = |channel, note| MidiMessage::NoteOn {
        channel,
        note,
        velocity: 0,
    };
    let deck = |state: &ControlState, side| state.side(side).active_deck.index() + 1;
    assert_eq!(
        (deck(&state, Side::Left), deck(&state, Side::Right)),
        (1, 2)
    );

    for note in [0x3C, 0x72] {
        for (side, main, alt) in [(Side::Left, 1u8, 3u8), (Side::Right, 2, 4)] {
            let (main_ch, alt_ch) = (main - 1, alt - 1);
            // DECK alt: control off on the old channel, on on the new one.
            send(off(main_ch, note), &mut state);
            send(on(alt_ch, note), &mut state);
            assert_eq!(deck(&state, side), usize::from(alt), "note {note:#x}");
            // A repeated press for the deck already shown must not toggle back.
            send(on(alt_ch, note), &mut state);
            assert_eq!(
                deck(&state, side),
                usize::from(alt),
                "repeat, note {note:#x}"
            );
            // The off message of the old deck alone changes nothing.
            send(off(main_ch, note), &mut state);
            assert_eq!(deck(&state, side), usize::from(alt));
            // DECK main.
            send(off(alt_ch, note), &mut state);
            send(on(main_ch, note), &mut state);
            assert_eq!(deck(&state, side), usize::from(main), "note {note:#x}");
            send(on(main_ch, note), &mut state);
            assert_eq!(deck(&state, side), usize::from(main));
        }
    }
    // The other side was never touched by the left side's presses.
    assert_eq!(
        (deck(&state, Side::Left), deck(&state, Side::Right)),
        (1, 2)
    );
    let _ = Input::Press;
}
