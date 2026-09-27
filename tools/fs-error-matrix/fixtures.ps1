# fs error-matrix fixtures for Windows (bug-694, bug-701): run from the probe's directory.
$ErrorActionPreference = 'Continue'
if (Test-Path fx) { icacls fx /reset /T /Q | Out-Null; Remove-Item -Recurse -Force fx }
New-Item -ItemType Directory fx | Out-Null
Set-Location fx
Set-Content -NoNewline -Path noread.txt -Value 'secret'
icacls noread.txt /deny "${env:USERNAME}:(R)" /Q | Out-Null
New-Item -ItemType Directory full\sub | Out-Null
Set-Content -NoNewline -Path full\f -Value 'x'
New-Item -ItemType Directory ro | Out-Null
icacls ro /deny "${env:USERNAME}:(W,AD)" /Q | Out-Null
[IO.File]::WriteAllBytes("$PWD\bad.txt", [byte[]](0xFF, 0xFE))
Set-Content -NoNewline -Path lines.txt -Value "a`nb"
Set-Content -NoNewline -Path plain.txt -Value 'file'
# Symbolic links need Developer Mode or elevation; a failure is reported, not fatal.
try { New-Item -ItemType SymbolicLink -Path link.txt -Target C:\Windows\win.ini -ErrorAction Stop | Out-Null } catch { Write-Output "fixture: no symlink privilege" }
try { New-Item -ItemType SymbolicLink -Path loopa -Target loopb -ErrorAction Stop | Out-Null; New-Item -ItemType SymbolicLink -Path loopb -Target loopa -ErrorAction Stop | Out-Null } catch { Write-Output "fixture: no symlink privilege (loop)" }
Set-Location ..
.\probe.exe
