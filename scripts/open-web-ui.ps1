param(
    [Parameter(Mandatory = $true)][string]$Url
)

# Opens the local web UI in a Chromium app window when Edge, Chrome or Brave is
# installed. An app window keeps only the Windows title bar: no address bar,
# tabs or bookmark bar, so the form gets the same vertical space as the desktop
# build. Other browsers keep their toolbar, and the page is opened normally.
$ErrorActionPreference = 'Stop'

$roots = @(
    ${env:ProgramFiles(x86)},
    $env:ProgramFiles,
    $env:LocalAppData
) | Where-Object { $_ }

$relative = @(
    'Microsoft\Edge\Application\msedge.exe',
    'Google\Chrome\Application\chrome.exe',
    'BraveSoftware\Brave-Browser\Application\brave.exe'
)

$browser = $null
foreach ($root in $roots) {
    foreach ($path in $relative) {
        $candidate = Join-Path $root $path
        if (Test-Path -LiteralPath $candidate) {
            $browser = $candidate
            break
        }
    }
    if ($browser) { break }
}

if ($browser) {
    Start-Process -FilePath $browser -ArgumentList @("--app=$Url")
} else {
    Start-Process -FilePath $Url
}
