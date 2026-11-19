# Pioneer DDJ-FLX4

Draft mapping, written on 2026-11-19 from Pioneer's official "DDJ-FLX4 List of MIDI messages"
(Ver 1.0, E1) and not yet confirmed by a capture: run `open-jockey midi capture --checklist
controllers/ddj-flx4/checklist.toml --out controllers/ddj-flx4/capture.toml` to check it. The
FLX4 has two decks and no layering (deck 1 on MIDI channel 1, deck 2 on channel 2), and the
mapping binds transport, loops (IN, OUT, 4 BEAT/EXIT, reloop, CUE/LOOP CALL as halve and
double), jog, tempo and tempo range, the mixer (trim, EQ, CFX as the filter, faders,
crossfader, headphone cue and mix), the browser and both LOAD buttons, and the Hot Cue, Beat
Jump, Sampler and Beat Loop (as loop roll) pads, with PLAY, CUE and CH CUE LEDs. Unbound,
because Open Jockey lacks the feature: the effects section (FX CH SELECT, FX SELECT, BEAT
arrows, LEVEL/DEPTH, FX ON/OFF), SMART CFX, SMART FADER, sync lock (BEAT SYNC long press),
loop in/out adjust, memory cues, fader start, the Pad FX, Keyboard and Key Shift pad modes,
mic level and the MONO/STEREO switch, and the LOADED, vinyl mode and level meter outputs.
MASTER and HEADPHONE LEVEL are left to the controller's own audio interface.
