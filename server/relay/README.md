# Investa Cloud Relay

Telegram과 로컬 Investa 사이에서 지시를 보관·전달하는 최소 Cloud Run 서비스입니다. 서버에는 브로커 주문 함수가 없고 `/healthz`도 `liveOrderEnabled: false`를 반환합니다.

## 보안 경계

- Telegram webhook secret header와 숫자 사용자 ID allowlist를 모두 검증합니다.
- 데스크톱 요청은 timestamp, nonce, method, path와 본문 해시를 HMAC-SHA256으로 서명합니다.
- nonce는 Firestore에 단일 생성해 재전송을 차단하고 TTL로 정리합니다.
- 작업 수신은 멱등 document ID, 임대는 Firestore update-time precondition을 사용합니다.
- Bot token, webhook secret과 desktop shared secret은 Secret Manager에만 둡니다.
- 서비스는 요청이 없으면 0개 인스턴스로 축소하고 최대 1개 인스턴스로 제한합니다.

## 로컬 검증

```powershell
node --test
```

외부 npm 패키지를 사용하지 않으므로 `npm install`은 필요하지 않습니다.

## Google Cloud 배포 순서

아래 예시의 프로젝트 ID, Telegram 사용자 ID와 secret 이름은 운영 환경에 맞게 바꿉니다. secret 원문을 셸 기록, 문서나 Git에 넣지 않습니다.

```powershell
gcloud config set project YOUR_PROJECT_ID
gcloud services enable run.googleapis.com cloudbuild.googleapis.com artifactregistry.googleapis.com firestore.googleapis.com secretmanager.googleapis.com
gcloud firestore databases create --location=asia-northeast3
gcloud iam service-accounts create investa-relay --display-name="Investa Cloud Relay"
gcloud projects add-iam-policy-binding YOUR_PROJECT_ID --member="serviceAccount:investa-relay@YOUR_PROJECT_ID.iam.gserviceaccount.com" --role="roles/datastore.user"
gcloud secrets create investa-telegram-bot-token --replication-policy=automatic
gcloud secrets create investa-telegram-webhook-secret --replication-policy=automatic
gcloud secrets create investa-desktop-shared-secret --replication-policy=automatic
gcloud secrets add-iam-policy-binding investa-telegram-bot-token --member="serviceAccount:investa-relay@YOUR_PROJECT_ID.iam.gserviceaccount.com" --role="roles/secretmanager.secretAccessor"
gcloud secrets add-iam-policy-binding investa-telegram-webhook-secret --member="serviceAccount:investa-relay@YOUR_PROJECT_ID.iam.gserviceaccount.com" --role="roles/secretmanager.secretAccessor"
gcloud secrets add-iam-policy-binding investa-desktop-shared-secret --member="serviceAccount:investa-relay@YOUR_PROJECT_ID.iam.gserviceaccount.com" --role="roles/secretmanager.secretAccessor"
gcloud firestore fields ttls update expiresAt --collection-group=relay_nonces --enable-ttl
gcloud run deploy investa-relay --source . --region=asia-northeast3 --service-account="investa-relay@YOUR_PROJECT_ID.iam.gserviceaccount.com" --allow-unauthenticated --min=0 --max=1 --concurrency=20 --cpu=1 --memory=256Mi --timeout=15s --set-env-vars="ALLOWED_TELEGRAM_USER_IDS=YOUR_NUMERIC_ID,MAX_REQUESTS_PER_MINUTE=60" --set-secrets="TELEGRAM_BOT_TOKEN=investa-telegram-bot-token:latest,TELEGRAM_WEBHOOK_SECRET=investa-telegram-webhook-secret:latest,DESKTOP_SHARED_SECRET=investa-desktop-shared-secret:latest"
```

`--allow-unauthenticated`는 Telegram이 Google IAM 토큰을 보낼 수 없기 때문에 필요합니다. 실제 webhook과 desktop endpoint는 애플리케이션 계층의 별도 secret/HMAC 검증을 통과해야 합니다.

배포 후에는 Telegram `setWebhook`에 Cloud Run의 `/telegram/webhook` URL과 `secret_token`을 설정합니다. Bot token과 secret 전송은 사용자 PC에서 직접 수행하며 채팅·스크린샷·로그에 남기지 않습니다.

## 비용 제한

- Cloud Run 최소 인스턴스 `0`, 최대 인스턴스 `1`
- Firestore에는 본문 최대 4,000자, 결과 최대 12,000자만 저장
- 요청 본문 기본 16 KiB, 분당 60회 제한
- Google Cloud Billing 예산 알림은 별도로 설정해야 하며 서비스 자체의 hard cap을 대신하지 않습니다.
