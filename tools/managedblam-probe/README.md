# ManagedBlam probe

Asks an editing kit's own `ManagedBlam.dll` whether it will load a tag.

This is the only oracle that answers the question a conversion actually raises.
Baboon re-reading its own output proves the file is self-consistent by Baboon's
rules; it says nothing about whether the mod tools accept it. Twice now a bug
has hidden in that gap.

## Build

`ManagedBlam.dll` is a mixed-mode assembly, so it needs the .NET **Framework**
compiler — PowerShell 7 and .NET Core cannot host it.

```powershell
$bin = "D:\SteamLibrary\steamapps\common\HREK\bin"
& "C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe" `
    -nologo -platform:x64 -reference:"$bin\ManagedBlam.dll" `
    -out:probe.exe probe.cs
```

## Run

A work list holds one tag per line as `<path without extension>|<extension>`,
relative to the kit's `tags` folder:

```
objects\weapons\rifle\assault_rifle|weapon
rasterizer\shaders\add|pixel_shader
```

Drive it. A tag that takes the engine down takes the probe with it, so the probe
writes and flushes one verdict at a time and the driver restarts it; the line
that never got written is the one that killed it, and it is recorded as `CRASH`
before the next attempt.

```powershell
.\drive.ps1 -Kit "D:\SteamLibrary\steamapps\common\HREK" `
            -WorkList work.txt -Results results.txt
```

Results are tab separated — a work line contains a `|` — as
`OK|FAIL|CRASH|DIED`, the work line, and the detail.

`results.txt.console.txt` holds the engine's own console. That is where the real
answer usually is: the crash callback only sees the shell's halt, while the
assertion, or an `AccessViolationException` inside native `tag_load`, prints
here first.

## One process per tag

`drive.ps1` loads everything in one process, which is fast and **lies**. The
engine accumulates state across loads, and a run of several hundred reports
crashes on tags that load perfectly well on their own: in one sweep, 65 of 69
"crashers" were this.

Use it to find candidates, then confirm them with `isolate.ps1`, which gives
each tag its own process. Slower, and the only verdict worth quoting.

```powershell
.\isolate.ps1 -Kit "D:\SteamLibrary\steamapps\common\HREK" `
               -WorkList work.txt -Results results.txt
```

## Reading a verdict

- **OK** — loaded, and its fields were walked. Not a claim that the contents are
  right, only that the engine accepts the file.
- **FAIL** — a managed exception. `could not be loaded` is a clean refusal.
- **CRASH** / **DIED** — the engine halted or the process died. Look in the
  console log.

Always run a control set of stock tags first. If those do not load, the harness
is wrong, not the tags.

## Notes

`reflect.cs` dumps the ManagedBlam type surface, so the probe can be written
against what the assembly declares rather than a guess at it. Build it the same
way and pass the kit's `bin` directory.

Both need an `AssemblyResolve` handler pointing at `<kit>\bin` and
`SetDllDirectory` on the same, installed before any `Bungie` type is touched —
which is why the real work sits in a `NoInlining` method, so the JIT does not
resolve those types too early.
