# Guard the portable Windows release: the shipped .exe files must not import
# the dynamic MSVC runtime, or they fail on a clean Windows install with
# "VCRUNTIME140.dll was not found". .cargo/config.toml enables the static CRT
# for x86_64-pc-windows-msvc; this script fails the build if that regresses.
#
# Usage: .\scripts\check-windows-runtime.ps1 path\to\pixiv-manager.exe [...]
$ErrorActionPreference = 'Stop'

if ($args.Count -eq 0) {
    throw 'usage: check-windows-runtime.ps1 <exe> [<exe> ...]'
}

# Imported DLL names are stored verbatim in the PE import directory, so a byte
# scan is enough and does not depend on dumpbin being on PATH.
$forbidden = @('VCRUNTIME140.dll', 'VCRUNTIME140_1.dll', 'MSVCP140.dll')

foreach ($file in $args) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
        throw "missing binary: $file"
    }
    $text = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($file))
    foreach ($dll in $forbidden) {
        if ($text.Contains($dll)) {
            throw "$file imports $dll; build with the static CRT (see .cargo/config.toml)"
        }
    }
    Write-Output "OK: $file has no dynamic MSVC runtime imports"
}
