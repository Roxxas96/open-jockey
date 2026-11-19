# Open Jockey — Windows test build

This is a pre-release for testing, not a release. It is unsigned and has no installer.

## 1. Start it

1. Unzip the folder anywhere, for example to `Documents\open-jockey`.
2. Double-click `Start FLX4.bat` or `Start GRV6.bat`, for the controller you plugged in.
3. Windows SmartScreen says "Windows protected your PC". Click **More info**, then **Run anyway**.
   It asks once.
4. If Windows says `VCRUNTIME140.dll` or `MSVCP140.dll` was not found, install Microsoft's
   "Visual C++ Redistributable" (x64, from microsoft.com), then start again.

Each `.bat` file starts Open Jockey with the configuration file next to it (`config-flx4.toml` or
`config-grv6.toml`), which picks that controller's mapping from the `controllers` folder.

The controller needs Pioneer's Windows audio driver (the one rekordbox and Serato use). The
sound goes to Windows' default output at first; pick the controller in Settings (the gear),
Audio, then restart from the `.bat` file.

## 2. Add your music

Open Jockey reads the music folders you give it. In this folder, click the address bar,
type `cmd` and press Enter, then:

```
open-jockey.exe library add-folder "C:\Users\<you>\Music"
```

It scans the folder; the window shows the tracks after its next start. You can also drag an
audio file from Explorer onto a deck.

## 3. If a control does the wrong thing: the capture

The FLX4 and GRV6 mappings are drafts made from Pioneer's MIDI lists. To check them, record
every control (in the same `cmd` window, controller plugged in, Open Jockey closed):

```
open-jockey.exe midi capture --checklist controllers\ddj-flx4\checklist.toml --out flx4-capture.toml
```

(`ddj-grv6` and `grv6-capture.toml` for the GRV6.) It asks for one control at a time: move it,
then press Enter. Send the `.toml` file it writes. A capture stopped halfway carries on where it
stopped when run again.

`open-jockey.exe midi monitor` prints every MIDI message the controller sends, which helps when
one control misbehaves.
