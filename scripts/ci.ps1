# All checks: tests, clippy, no_global_oom_handling build, Miri (reduced proptest cases).
$ErrorActionPreference = 'Stop'
$env:HTTPS_PROXY = $null
$env:HTTP_PROXY = $null

function Step($name, [scriptblock]$cmd) {
    Write-Host "== $name"
    & $cmd
    if ($LASTEXITCODE -ne 0) { Write-Host "FAILED: $name"; exit $LASTEXITCODE }
}

Step 'test' { cargo test }
Step 'clippy' { cargo clippy --all-targets --all-features -- -D warnings }
Step 'no-oom' { & "$PSScriptRoot\check-no-oom.ps1" }
$env:MIRIFLAGS = '-Zmiri-disable-isolation'
$env:PROPTEST_CASES = '1'
$env:PROPTEST_DISABLE_FAILURE_PERSISTENCE = '1'
# linux target: Windows `System` reads its header outside the Box under the test's #[global_allocator],
# which Stacked Borrows rejects; the full fault-injection runs take hours under Miri
Step 'miri' { cargo +nightly miri test --target x86_64-unknown-linux-gnu -- --skip try_insert_is_atomic --skip vacant_entry_try_insert_is_atomic }
Write-Host 'ci: OK'
