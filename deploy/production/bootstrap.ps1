[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$AdminPasswordHash,
    [string]$ProviderApiKey = ""
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$Secrets = Join-Path $Root "secrets"
$EnvFile = Join-Path $Root ".env"

if ($AdminPasswordHash -notmatch '^\$(argon2id|2[aby])\$') {
    throw "AdminPasswordHash must be a Caddy-supported Argon2id or bcrypt hash; plaintext is rejected."
}

New-Item -ItemType Directory -Force $Secrets | Out-Null
if (-not (Test-Path $EnvFile)) {
    Copy-Item (Join-Path $Root ".env.example") $EnvFile
}

$bytes = [byte[]]::new(32)
$rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
try {
    $rng.GetBytes($bytes)
} finally {
    $rng.Dispose()
}
$accessKey = [Convert]::ToBase64String($bytes).TrimEnd('=').Replace('+', '-').Replace('/', '_')

[System.IO.File]::WriteAllText((Join-Path $Secrets "engine_access_key"), $accessKey, [Text.UTF8Encoding]::new($false))
[System.IO.File]::WriteAllText((Join-Path $Secrets "admin_password_hash"), $AdminPasswordHash, [Text.UTF8Encoding]::new($false))
[System.IO.File]::WriteAllText((Join-Path $Secrets "provider_api_key"), $ProviderApiKey, [Text.UTF8Encoding]::new($false))

Write-Host "Created production secret files and .env (if absent)."
Write-Host "Review .env, then run: docker compose --env-file .env -f compose.yaml up -d --build"
