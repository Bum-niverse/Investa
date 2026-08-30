param(
    [string]$ProjectId = "investa-remote-bumniverse",
    [string]$Region = "asia-northeast3",
    [string]$Service = "investa-relay"
)

$ErrorActionPreference = "Stop"
if (-not (Get-Command gcloud -ErrorAction SilentlyContinue)) {
    throw "gcloud CLI가 없습니다. Google Cloud Shell에서 이 스크립트를 실행하세요."
}
if ($ProjectId -notmatch '^[a-z][a-z0-9-]{4,28}[a-z0-9]$') {
    throw "Google Cloud project ID 형식이 올바르지 않습니다."
}

Write-Output "Cloud Run ingress/authentication 상태"
gcloud run services describe $Service --project $ProjectId --region $Region --format="yaml(metadata.name,metadata.annotations,spec.template.spec.serviceAccountName,status.url)"

Write-Output "프로젝트 IAM 역할"
gcloud projects get-iam-policy $ProjectId --flatten="bindings[].members" --format="table(bindings.role,bindings.members)"

Write-Output "Firestore TTL 상태"
foreach ($collection in @("relay_jobs", "relay_nonces")) {
    gcloud firestore fields ttls list --project $ProjectId --collection-group $collection --format="table(collectionGroup,field,state)"
}

Write-Output "Secret Manager 비밀 목록과 복제 정책(값은 출력하지 않음)"
gcloud secrets list --project $ProjectId --format="table(name,replication.locations.list():label=LOCATIONS,createTime)"

Write-Output "이 스크립트는 읽기 전용입니다. 공개 URL, IAM, TTL 또는 비밀 버전을 자동 변경하지 않습니다."
