# Installe Echoes sur Windows (PowerShell).
#   Première machine :   irm https://raw.githubusercontent.com/arthurreeb2-tech/Echoes/main/install.ps1 | iex
#   Machines suivantes : $env:ECHOES_CODE="<code>"; irm https://raw.githubusercontent.com/arthurreeb2-tech/Echoes/main/install.ps1 | iex
$ErrorActionPreference = "Stop"
$repo = "arthurreeb2-tech/Echoes"
$asset = "echoes-x86_64-pc-windows-msvc.exe"
$url = "https://github.com/$repo/releases/latest/download/$asset"
$tmp = Join-Path $env:TEMP "echoes-install.exe"
Invoke-WebRequest $url -OutFile $tmp -UseBasicParsing
$expected = ((Invoke-WebRequest "$url.sha256" -UseBasicParsing).Content -split '\s+')[0].ToLower()
$actual = (Get-FileHash $tmp -Algorithm SHA256).Hash.ToLower()
if ($expected -ne $actual) { throw "Echoes : empreinte incorrecte, téléchargement refusé." }
if ($env:ECHOES_CODE) { & $tmp install $env:ECHOES_CODE } else { & $tmp install }
Remove-Item $tmp -ErrorAction SilentlyContinue
