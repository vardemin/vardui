# Install the latest `vard` release into ~\.vard\bin (or $env:VARD_INSTALL_DIR).
#   irm https://raw.githubusercontent.com/vardemin/vardui/master/install.ps1 | iex
$ErrorActionPreference = "Stop"
$repo = "vardemin/vardui"
$dir = if ($env:VARD_INSTALL_DIR) { $env:VARD_INSTALL_DIR } else { Join-Path $HOME ".vard\bin" }
$url = "https://github.com/$repo/releases/latest/download/vard-x86_64-pc-windows-msvc.zip"
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("vard-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force $tmp | Out-Null
try {
    Invoke-WebRequest $url -OutFile "$tmp\vard.zip" -UseBasicParsing
    Invoke-WebRequest "$url.sha256" -OutFile "$tmp\vard.zip.sha256" -UseBasicParsing
    $want = ((Get-Content "$tmp\vard.zip.sha256") -split '\s+')[0].ToLower()
    $got = (Get-FileHash "$tmp\vard.zip" -Algorithm SHA256).Hash.ToLower()
    if ($want -ne $got) { throw "checksum mismatch for $url" }
    New-Item -ItemType Directory -Force $dir | Out-Null
    Expand-Archive "$tmp\vard.zip" -DestinationPath $dir -Force
    Write-Output "installed $(& "$dir\vard.exe" --version) to $dir\vard.exe"
    $path = [Environment]::GetEnvironmentVariable("Path", "User")
    if (-not ($path -split ';' -contains $dir)) {
        [Environment]::SetEnvironmentVariable("Path", "$path;$dir", "User")
        Write-Output "added $dir to your user PATH (open a new terminal)"
    }
} finally {
    Remove-Item -Recurse -Force $tmp
}
