<#
Sets Mixxx up on Windows to send its decks to Face2Face DJ.

  1. Copies the Face2Face DJ mapping into Mixxx's user mapping folder.
  2. Switches the "Face2Face DJ" controller on in Mixxx's settings, with that
     mapping (a backup of the settings file is kept next to it).
  3. Makes sure the MIDI port "Face2Face DJ" exists. Windows has no built-in
     way to create one, so this uses loopMIDI (free, by Tobias Erichsen):
     the script installs it if you agree, opens it, and waits while you add
     the port. loopMIDI has no command to add a port, so that one step is by hand.

Run it by double-clicking "Setup Mixxx (Windows).cmd" in this folder.
It changes nothing outside Mixxx's settings folder, and nothing in your library.

-SettingsDir    Mixxx's settings folder, if it is not %LOCALAPPDATA%\Mixxx.
-SkipLoopMidi   Do steps 1 and 2 only, and report whether the port exists.
#>
param(
    [string]$SettingsDir = (Join-Path $env:LOCALAPPDATA 'Mixxx'),
    [switch]$SkipLoopMidi
)

$ErrorActionPreference = 'Stop'
$PortName = 'Face2Face DJ'
# Mixxx's name for the device in its settings: spaces become underscores.
$DeviceKey = 'Face2Face_DJ'
$MappingFiles = 'Face2Face-DJ.midi.xml', 'Face2Face-DJ.js'

function Say($text) { Write-Host $text }
function Step($text) { Write-Host ''; Write-Host "== $text" -ForegroundColor Cyan }

# ---- MIDI ports, as Windows lists them --------------------------------------

Add-Type -Namespace Face2Face -Name Midi -MemberDefinition @'
[StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
public struct MidiInCaps {
    public ushort wMid;
    public ushort wPid;
    public uint vDriverVersion;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)]
    public string szPname;
    public uint dwSupport;
}
[DllImport("winmm.dll")]
public static extern uint midiInGetNumDevs();
[DllImport("winmm.dll", CharSet = CharSet.Unicode, EntryPoint = "midiInGetDevCapsW")]
public static extern uint midiInGetDevCaps(UIntPtr deviceId, ref MidiInCaps caps, uint size);
'@

function Get-MidiInputNames {
    $names = @()
    $count = [Face2Face.Midi]::midiInGetNumDevs()
    for ($i = 0; $i -lt $count; $i++) {
        $caps = New-Object Face2Face.Midi+MidiInCaps
        $size = [System.Runtime.InteropServices.Marshal]::SizeOf($caps)
        if ([Face2Face.Midi]::midiInGetDevCaps([UIntPtr][uint32]$i, [ref]$caps, $size) -eq 0) {
            $names += $caps.szPname
        }
    }
    return $names
}

function Test-Port { return (Get-MidiInputNames) -contains $PortName }

# ---- Mixxx's settings file ---------------------------------------------------

# Sets "key value" in [section] of an ini-style list of lines, replacing an
# existing line for that key and adding the section if it is missing.
function Set-ConfigValue([System.Collections.Generic.List[string]]$lines, [string]$section, [string]$key, [string]$value) {
    $header = "[$section]"
    $start = $lines.IndexOf($header)
    if ($start -lt 0) {
        if ($lines.Count -gt 0 -and $lines[$lines.Count - 1] -ne '') { $lines.Add('') }
        $lines.Add($header)
        $lines.Add("$key $value")
        return
    }
    for ($i = $start + 1; $i -lt $lines.Count -and -not $lines[$i].StartsWith('['); $i++) {
        if ($lines[$i].StartsWith("$key ")) {
            $lines[$i] = "$key $value"
            return
        }
    }
    $lines.Insert($start + 1, "$key $value")
}

# ---- 1 and 2: mapping and settings -------------------------------------------

Step 'Mixxx'
if (Get-Process -Name 'mixxx' -ErrorAction SilentlyContinue) {
    Say 'Mixxx is running. Close it first: it rewrites its settings when it closes,'
    Say 'which would undo this set-up. Then run this again.'
    exit 1
}

$controllers = Join-Path $SettingsDir 'controllers'
New-Item -ItemType Directory -Force -Path $controllers | Out-Null
foreach ($file in $MappingFiles) {
    $source = Join-Path $PSScriptRoot $file
    if (-not (Test-Path $source)) {
        Say "Missing $file next to this script. Keep the files of this folder together."
        exit 1
    }
    Copy-Item -Force $source $controllers
}
Say "Mapping copied to $controllers"

$config = Join-Path $SettingsDir 'mixxx.cfg'
if (Test-Path $config) {
    $backup = "$config.before-face2face"
    if (-not (Test-Path $backup)) { Copy-Item $config $backup }
    # Mixxx reads and writes UTF-8 without a byte-order mark, with / in paths.
    $encoding = New-Object System.Text.UTF8Encoding($false)
    $lines = New-Object 'System.Collections.Generic.List[string]'
    $lines.AddRange([System.IO.File]::ReadAllLines($config, $encoding))
    $mapping = (Join-Path $controllers $MappingFiles[0]) -replace '\\', '/'
    Set-ConfigValue $lines 'ControllerPreset' $DeviceKey $mapping
    Set-ConfigValue $lines 'Controller' $DeviceKey '1'
    [System.IO.File]::WriteAllLines($config, $lines, $encoding)
    Say 'Controller "Face2Face DJ" switched on in Mixxx''s settings.'
    Say "(Backup of the settings: $backup)"
} else {
    Say 'Mixxx has no settings file yet. Start Mixxx once, close it, and run this again,'
    Say 'or switch the controller on by hand: Preferences > Controllers > Face2Face DJ >'
    Say 'Enabled, Load Mapping "Face2Face DJ".'
}

# ---- 3: the MIDI port ---------------------------------------------------------

Step "MIDI port ""$PortName"""
if (Test-Port) {
    Say 'The port exists.'
} elseif ($SkipLoopMidi) {
    Say 'The port does not exist (loopMIDI step skipped).'
} else {
    $loopMidi = @(
        (Join-Path ${env:ProgramFiles(x86)} 'Tobias Erichsen\loopMIDI\loopMIDI.exe'),
        (Join-Path $env:ProgramFiles 'Tobias Erichsen\loopMIDI\loopMIDI.exe')
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1

    if (-not $loopMidi) {
        Say 'loopMIDI is not installed. It is the free tool that provides the MIDI port.'
        $winget = Get-Command winget -ErrorAction SilentlyContinue
        if ($winget -and (Read-Host 'Install it now with winget? (y/n)') -eq 'y') {
            winget install --id TobiasErichsen.loopMIDI --exact --accept-source-agreements --accept-package-agreements
        } else {
            Say 'Opening its download page. Install it, then run this again.'
            Start-Process 'https://www.tobias-erichsen.de/software/loopmidi.html'
            exit 1
        }
        $loopMidi = Join-Path ${env:ProgramFiles(x86)} 'Tobias Erichsen\loopMIDI\loopMIDI.exe'
        if (-not (Test-Path $loopMidi)) {
            Say 'loopMIDI was not found after installing. Start it from the Start menu, then run this again.'
            exit 1
        }
    }

    if (-not (Get-Process -Name 'loopMIDI' -ErrorAction SilentlyContinue)) { Start-Process $loopMidi }
    Set-Clipboard -Value $PortName
    Say ''
    Say 'In the loopMIDI window (it may be an icon near the clock; double-click it):'
    Say "  1. Click in ""New port-name"" and paste (Ctrl+V). The name is: $PortName"
    Say '  2. Click the + button.'
    Say 'Waiting for the port...'
    $deadline = (Get-Date).AddMinutes(5)
    while (-not (Test-Port)) {
        if ((Get-Date) -gt $deadline) {
            Say 'No port after 5 minutes. Add it in loopMIDI, then run this again to check.'
            exit 1
        }
        Start-Sleep -Seconds 1
    }
    Say 'The port exists.'
    Say 'Tip: right-click the loopMIDI icon near the clock and tick the autostart'
    Say 'option, so the port is there after every restart of the PC.'
}

Step 'Done'
Say 'Now: start Face2Face DJ and choose Mixxx as the source, then start Mixxx.'
Say 'The status in Face2Face DJ changes from "waiting for Mixxx" to "running".'
