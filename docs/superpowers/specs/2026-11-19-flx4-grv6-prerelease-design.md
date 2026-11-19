# FLX4 and GRV6 mappings, and a Windows pre-release — design

Approved by the owner on 2026-11-19, with a lighter process for one evening (see Process).

## Intent

A friend of the owner plays Open Jockey tonight, at 21:00, on his own Windows 11 computer, with
his Pioneer DDJ-FLX4 and DDJ-GRV6. The owner talks him through it remotely. "Done" means he plays
with both controllers before the call ends, not just that his controls are recorded.

## Goals

- **A Windows pre-release** he can download: a GitHub pre-release holding a zip with
  `open-jockey.exe` (and any DLL it needs), built by a workflow. The owner tests it on his own
  Windows machine before the call.
- **Two mappings**, `controllers/ddj-flx4/mapping.toml` and `controllers/ddj-grv6/mapping.toml`,
  drafted before the call from Pioneer's official MIDI message lists. They follow the project's
  standard: VirtualDJ's default layout for that controller, limited to the actions Open Jockey
  has, like the SB3 mapping. Controls for features Open Jockey lacks stay unbound.
- **A capture checklist for each** (`checklist.toml`), so that `open-jockey midi capture` on the
  call confirms the draft and finds what is wrong.
- **A setup sheet** for the friend: download, unzip, SmartScreen ("More info" → "Run anyway"),
  where `config.toml` goes, the `controller.mapping` line pointing at the mapping file, the audio
  device, and how to run the capture.

## Non-goals

- Making the two mappings built-ins tonight. They load from a file path
  (`controller.mapping = "C:/.../mapping.toml"`). Built-ins come later, after his capture.
- Signed binaries, an installer, macOS or Linux pre-releases.
- The release itself (plan 13).
- LED feedback beyond what the SB3 mapping does.

## Anti-goals

- **No MIDI value from anywhere but Pioneer's lists or his capture.** No values from other apps'
  mappings (Mixxx, djay, community files). Where a list is missing or unclear, the binding waits
  for the capture.
- **No change to how the SB3 behaves.**
- **The pre-release must say what it is:** a pre-release for testing, not a release.

## Sources

- GRV6: `DDJ-GRV6_MIDI_Message_List_E1.pdf` from downloads.support.alphatheta.com (4 pages).
- FLX4: the owner downloads the list from AlphaTheta's support site (it refuses scripts).

## Decided

- **Owner:** the friend's computer is Windows 11. He plays tonight. Mappings follow the SB3's
  standard and are adapted later if needed. A pre-release on GitHub, tested first on the owner's
  Windows machine.
- **Owner:** drafts come from Pioneer's lists. This changes the SB3 rule (capture only) for these
  two controllers; the capture still has the last word.
- **My call:** the pre-release is triggered by hand (`workflow_dispatch`) or a `v*-pre*` tag, runs
  on `windows-latest`, builds `cargo build --release --locked -p open-jockey`, and zips the exe
  with every DLL next to it in `target/release`, `LICENSE` and the setup sheet. The version tag
  is `v0.0.0-pre.1`.
- **My call:** layering (hardware or software decks) and SHIFT handling are read from each list,
  as the SB3 mapping's header explains its own.

## Process (owner-approved for tonight)

No plan document and no per-task reviews. Order: pre-release first, FLX4, GRV6, checklists,
setup sheet. One review of the whole branch at the end. The call is the hands-on check. After
it: replay tests from his captures (like `sb3_replay.rs`), built-ins, and a normal PR.

## Verification

- The mappings load: `open-jockey midi monitor --controller <path>` parses them, and a test parses
  both files.
- `cargo test --workspace`, clippy and deny pass; the pre-release workflow succeeds.
- The owner runs the pre-release on his Windows machine; the friend plays on the call.
