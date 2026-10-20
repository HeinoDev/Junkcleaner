# Junkcleaner (PowerShell) - no install needed, just run it.
# Usage:  .\junkcleaner.ps1          (scan only)
#         .\junkcleaner.ps1 -Clean   (delete what was found)
param([switch]$Clean)

$local = $env:LOCALAPPDATA
$roaming = $env:APPDATA

$targets = [ordered]@{
    "Temporary User Files"     = @($env:TEMP)
    "Windows Temporary Files"  = @("$env:SystemRoot\Temp")
    "DirectX Shader Cache"     = @("$local\D3DSCache")
    "FiveM Cache"              = @("$local\FiveM\FiveM.app\data\cache",
                                   "$local\FiveM\FiveM.app\data\server-cache",
                                   "$local\FiveM\FiveM.app\data\server-cache-priv")
    "Discord Cache"            = @("$roaming\discord\Cache",
                                   "$roaming\discord\Code Cache",
                                   "$roaming\discord\GPUCache")
}

$total = 0
foreach ($name in $targets.Keys) {
    $size = 0
    foreach ($path in $targets[$name]) {
        if (-not (Test-Path $path)) { continue }
        $items = Get-ChildItem $path -Force -ErrorAction SilentlyContinue
        $size += ($items | Get-ChildItem -Recurse -Force -File -ErrorAction SilentlyContinue |
                  Measure-Object Length -Sum).Sum
        if ($Clean) {
            # Files in use are skipped
            $items | Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
    $total += $size
    "{0,-26} {1,10:N1} MB" -f $name, ($size / 1MB)
}
"{0,-26} {1,10:N1} MB" -f "Total", ($total / 1MB)
if (-not $Clean) { "Scan only. Run with -Clean to delete." }
