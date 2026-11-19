# Pioneer DDJ-GRV6

Draft mapping, written on 2026-11-19 from Pioneer's official "DDJ-GRV6 MIDI Message List" (E1)
and not yet confirmed by a capture: run `open-jockey midi capture --checklist
controllers/ddj-grv6/checklist.toml --out controllers/ddj-grv6/capture.toml` to check it. The
GRV6 is hardware layered (decks 1-4 send on MIDI channels 1-4), and the mapping binds
transport, loops, jog, tempo, the mixer (trim, EQ, SOUND COLOR FX as the filter, faders,
crossfader, headphone cue and mix), the browser and LOAD 1-4, and the Hot Cue, Stems (pads 1-4),
Beat Jump, Sampler and Beat Loop (as loop roll) pads, with PLAY, CUE and CH CUE LEDs.
Unbound, because Open Jockey lacks the feature: the GROOVE CIRCUIT (gain, drum swap, capture,
drum roll, drum release), Beat FX and the SOUND COLOR FX on/off button, QUANTIZE, KEY SYNC,
MEMORY and memory-cue call, fader start, the Keyboard, Pad FX and Key Shift pad modes, booth
and mic levels, DISCOVER, BACK, PREVIEW and the selector tilts, and the LOAD and jog
illumination. MASTER and HEADPHONES LEVEL are left to the controller's own audio interface.
