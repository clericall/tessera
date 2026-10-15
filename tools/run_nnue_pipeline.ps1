# Run the full NNUE loop end to end: self-play PGN -> training data -> net ->
# engine/python parity -> match against the hand-crafted eval.
#
#   powershell -File tools/run_nnue_pipeline.ps1 -Pgn selfplay.pgn [-Games 400] [-Match 8]
#
# Assumes fastchess, the compiler and a built engine are already available; see
# docs/milestones/M12-selfplay-data.md and M13-nnue-training.md.

param(
    [string]$Pgn = "selfplay.pgn",
    [string]$Data = "selfplay.bin",
    [string]$Net = "engine/nnue-smoke.tnn",
    [int]$Games = 0,
    [int]$Rounds = 400,
    [int]$Match = 8,
    [int]$Hidden = 32,
    [int]$Epochs = 60
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$fastchess = "C:\Users\kolaimn\tools\fastchess\fastchess-windows-x86-64\fastchess.exe"
$engine = "compiler\samples\bbuci.exe"

Write-Host "== building the engine =="
& "compiler\target\debug\tesserac.exe" build "engine\bb.ter" -o "samples\bbuci.exe" --tier c
if ($LASTEXITCODE -ne 0) { throw "engine build failed" }

if ($Games -gt 0) {
    Write-Host "== self-play: $Games games =="
    $cfg = Get-Content compiler\config-selfplay.json -Raw | ConvertFrom-Json
    $cfg.games = 1
    $cfg.rounds = $Games
    $cfg | ConvertTo-Json -Depth 12 | Set-Content compiler\config-selfplay.json -Encoding UTF8
    & $fastchess -config file=compiler/config-selfplay.json -pgnout "file=$Pgn" | Out-Null
}

Write-Host "== converting PGN to training records =="
& python tools\gen_data.py --selftest
& python tools\gen_data.py --pgn $Pgn --out $Data --min-plies 8

Write-Host "== training =="
& python tools\train.py --data $Data --out $Net --hidden $Hidden --epochs $Epochs --lr 0.002 --batch 512 --weight-decay 1e-4

Write-Host "== engine / python parity =="
& python tools\train.py --selftest
$fens = @(
    "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
    "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
    "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8"
)
$agree = 0
foreach ($fen in $fens) {
    $py = python tools\train.py --out $Net --check-fen $fen | Select-Object -Last 1
    $script = "setoption name EvalFile value $Net`nposition fen $fen`neval`nquit`n"
    $line = ($script | & ".\compiler\samples\bbuci.exe" | Select-String 'hce=').Line
    $en = ($line -replace '.*nnue=(-?\d+).*', '$1')
    if ($py -eq $en) { $agree++ } else { Write-Host "  MISMATCH $fen python=$py engine=$en" -ForegroundColor Red }
}
Write-Host "  parity: $agree/$($fens.Count)"

Write-Host "== match: NNUE vs hand-crafted eval =="
$cfg = Get-Content compiler\config.json -Raw | ConvertFrom-Json
$a = $cfg.engines[0].PSObject.Copy()
$a.name = 'Tessera-NNUE'
$a.options = @(@('Threads', '1'), @('Hash', '16'), @('EvalFile', ($Net -replace '\\', '/')))
$b = $cfg.engines[0].PSObject.Copy()
$b.name = 'Tessera-HCE'
$b.options = @(@('Threads', '1'), @('Hash', '16'))
$cfg.engines = @($a, $b)
$cfg.games = 2
$cfg.rounds = [math]::Max(2, [int]($Match / 2))
$cfg.PSObject.Properties.Remove('stats')
$cfg | ConvertTo-Json -Depth 12 | Set-Content compiler\config-ab.json -Encoding UTF8
& $fastchess -config file=compiler/config-ab.json