# Builds the library against alloc compiled with no_global_oom_handling.
# Any infallible allocation (push, insert, clone, vec!, format!, Box::new...) fails to compile.
$ErrorActionPreference = 'Stop'
$env:HTTPS_PROXY = $null
$env:HTTP_PROXY = $null
$env:RUSTFLAGS = '--cfg no_global_oom_handling'
cargo +nightly build --lib '-Zbuild-std=core,alloc' --target x86_64-pc-windows-msvc --target-dir target/no-oom
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Write-Host 'no_global_oom_handling: OK'
