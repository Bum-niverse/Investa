[CmdletBinding()]
param(
  [Parameter(Mandatory = $true, Position = 0)]
  [string]$GcloudExecutable,

  [Parameter(Position = 1, ValueFromRemainingArguments = $true)]
  [string[]]$GcloudArguments
)

$ErrorActionPreference = 'Stop'

if (-not [System.IO.Path]::IsPathRooted($GcloudExecutable)) {
  throw 'Google Cloud CLI path must be absolute.'
}

$extension = [System.IO.Path]::GetExtension($GcloudExecutable)
if ($extension -notin @('.cmd', '.bat', '.exe')) {
  throw 'Google Cloud CLI path has an unsupported extension.'
}

if (-not (Test-Path -LiteralPath $GcloudExecutable -PathType Leaf)) {
  throw 'Google Cloud CLI executable was not found.'
}

& $GcloudExecutable @GcloudArguments
if ($null -eq $LASTEXITCODE) {
  exit 1
}
exit $LASTEXITCODE
