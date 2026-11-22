param(
    [switch]$Release,
    [switch]$AarOnly,
    [switch]$Install,
    [string]$Target = "v2rayNG\v2rayNG-2.3.15",
    [string]$Device = "",
    [string]$Proxy = ""
)

$ErrorActionPreference = "Stop"
$LibRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$AarProj = Join-Path $LibRoot "android-aar"
$AarJniLibs = Join-Path $AarProj "libv2ray\src\main\jniLibs"
$DistDir = Join-Path $LibRoot "dist"

if (-not $env:ANDROID_NDK_HOME) {
    $env:ANDROID_NDK_HOME = "C:\Users\abc15\AppData\Local\Android\Sdk\ndk\29.0.14206865"
}

$gradleProxyArgs = @()
if ($Proxy) {
    $parts = $Proxy.Split(":")
    if ($parts.Length -eq 2) {
        $gradleProxyArgs = @(
            "-Dhttp.proxyHost=$($parts[0])", "-Dhttp.proxyPort=$($parts[1])",
            "-Dhttps.proxyHost=$($parts[0])", "-Dhttps.proxyPort=$($parts[1])"
        )
    }
}

Write-Host "=== [1/4] Building arm64-v8a libxray_android.so (Release=$Release, NDK=$env:ANDROID_NDK_HOME) ===" -ForegroundColor Cyan
Push-Location (Join-Path $LibRoot "rust-xray-jni")
try {
    $ndkArgs = @("ndk", "-t", "arm64-v8a", "-P", "24", "-o", $AarJniLibs, "build")
    if ($Release) { $ndkArgs += "--release" }
    & cargo @ndkArgs
    if ($LASTEXITCODE -ne 0) { throw "cargo ndk build failed" }
} finally {
    Pop-Location
}

Write-Host "=== [2/4] Packaging standard libv2ray.aar (Gradle 9.8.2 + AGP 9.4.1 + Kotlin 2.4.21 + SDK 37) ===" -ForegroundColor Cyan
Push-Location $AarProj
try {
    & .\gradlew.bat :libv2ray:assembleRelease @gradleProxyArgs
    if ($LASTEXITCODE -ne 0) { throw "AAR Gradle build failed" }
} finally {
    Pop-Location
}

New-Item -ItemType Directory -Force -Path $DistDir | Out-Null
$BuiltAar = Join-Path $AarProj "libv2ray\build\outputs\aar\libv2ray-release.aar"
$OutAar = Join-Path $DistDir "libv2ray.aar"
Copy-Item $BuiltAar $OutAar -Force
Write-Host ("Built AAR: $OutAar (" + [math]::Round((Get-Item $OutAar).Length / 1MB, 2) + " MB)") -ForegroundColor Green

if ($AarOnly) {
    return
}

$V2rayNgRoot = Join-Path (Split-Path -Parent $LibRoot) "$Target\V2rayNG"
$V2rayNgLibs = Join-Path $V2rayNgRoot "app\libs"
New-Item -ItemType Directory -Force -Path $V2rayNgLibs | Out-Null
Copy-Item $OutAar (Join-Path $V2rayNgLibs "libv2ray.aar") -Force
Write-Host "Copied libv2ray.aar -> $V2rayNgLibs (zero frontend code modified!)" -ForegroundColor Cyan

Write-Host "=== [3/4] Building $Target APK ===" -ForegroundColor Cyan
Push-Location $V2rayNgRoot
try {
    $task = if ($Release) { "assembleFdroidRelease" } else { "assembleFdroidDebug" }
    $skip = if ($Release) { "checkFdroidReleaseAarMetadata" } else { "checkFdroidDebugAarMetadata" }
    & .\gradlew.bat $task -x $skip @gradleProxyArgs
    if ($LASTEXITCODE -ne 0) { throw "APK Gradle build failed" }
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
