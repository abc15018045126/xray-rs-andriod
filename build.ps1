param(
    [switch]$Release,
    [switch]$Install,
    [string]$Device = ""
)

$ErrorActionPreference = "Stop"
$LibRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$V2rayNgApp = Join-Path (Split-Path -Parent $LibRoot) "v2rayNG-master\V2rayNG\app\src\main"
$V2rayNgRoot = Join-Path (Split-Path -Parent $LibRoot) "v2rayNG-master\V2rayNG"

if (-not $env:ANDROID_NDK_HOME) {
    $env:ANDROID_NDK_HOME = "C:\Users\abc15\AppData\Local\Android\Sdk\ndk\27.1.12297006"
}

Write-Host "=== [1/4] Building arm64-v8a libxray_android.so (Release=$Release) ===" -ForegroundColor Cyan
Push-Location (Join-Path $LibRoot "rust-xray-jni")
try {
    $ndkArgs = @("ndk", "-t", "arm64-v8a", "-P", "24", "-o", (Join-Path $V2rayNgApp "jniLibs"), "build")
    if ($Release) { $ndkArgs += "--release" }
    & cargo @ndkArgs
    if ($LASTEXITCODE -ne 0) { throw "cargo ndk build failed" }
} finally {
    Pop-Location
}

Write-Host "=== [2/4] Syncing assets & libv2ray Kotlin bindings into v2rayNG ===" -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path (Join-Path $V2rayNgApp "assets"), (Join-Path $V2rayNgApp "java\libv2ray"), (Join-Path $V2rayNgApp "java\go") | Out-Null
Copy-Item (Join-Path $LibRoot "assets\*") (Join-Path $V2rayNgApp "assets\") -Force
Copy-Item (Join-Path $LibRoot "kotlin\libv2ray\*.kt") (Join-Path $V2rayNgApp "java\libv2ray\") -Force
Copy-Item (Join-Path $LibRoot "kotlin\go\Seq.kt") (Join-Path $V2rayNgApp "java\go\") -Force

Write-Host "=== [3/4] Building v2rayNG APK ===" -ForegroundColor Cyan
Push-Location $V2rayNgRoot
try {
    $task = if ($Release) { "assembleFdroidRelease" } else { "assembleFdroidDebug" }
    $skip = if ($Release) { "checkFdroidReleaseAarMetadata" } else { "checkFdroidDebugAarMetadata" }
    & .\gradlew.bat $task -x $skip
    if ($LASTEXITCODE -ne 0) { throw "Gradle build failed" }
} finally {
    Pop-Location
}

if ($Install) {
    Write-Host "=== [4/4] Installing APK via ADB ===" -ForegroundColor Cyan
    $variant = if ($Release) { "release" } else { "debug" }
    $apk = Get-ChildItem (Join-Path $V2rayNgRoot "app\build\outputs\apk\fdroid\$variant\*arm64-v8a*.apk") | Select-Object -First 1
    if (-not $apk) { throw "APK not found in fdroid\$variant" }
    $adbArgs = @()
    if ($Device) { $adbArgs += @("-s", $Device) }
    $adbArgs += @("install", "-r", $apk.FullName)
    & adb @adbArgs
    if ($LASTEXITCODE -ne 0) { throw "adb install failed" }
    Write-Host "Installed: $($apk.Name)" -ForegroundColor Green
}
