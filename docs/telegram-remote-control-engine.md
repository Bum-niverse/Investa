# Telegram 원격운영 엔진

## 목적

Telegram을 종목별 단축 명령 모음이 아니라 Investa 전체 업무를 원격으로 요청하고 결과를 확인하는 운영 채널로 사용한다. 로컬 코어와 Cloud Run·Firestore 어댑터, 운영 Secret 주입까지 구현했지만 공개 `run.app` 라우팅 404 때문에 Bot webhook 왕복은 아직 열지 않았다. Gemini 연결은 선택형 어댑터로 남겨 둔다.

## 현재 구현

- `status`, `analysis`, `meeting`, `paper_order_proposal`, `shadow_control`, `system_control`의 결정론적 명령 분류
- 최대 10개의 Telegram 숫자 사용자 ID allowlist
- 1~4,000자 입력, 제어문자와 비밀정보 표식 차단
- source·request ID와 전체 요청 해시를 사용한 멱등 재전송 처리
- 24시간보다 오래됐거나 미래 시각이 비정상인 요청 차단
- SQLite STRICT 작업 원장과 수신·큐·승인·거절·취소 사건 원장
- 분석·상태·회의는 `queued`, 투자·자동매매·시스템 제어는 `awaiting_local_approval`
- 로컬 사용자의 승인·거절·취소 상태 전이
- `liveOrderEnabled=false`, 전송·AI 공급자 미연결 상태의 명시적 진단
- 의존성 없는 Node 22 Cloud Run relay, Firestore 작업 큐와 만료 임대 복구
- Telegram webhook secret·numeric user allowlist·멱등 update ID 검증
- desktop HMAC-SHA256 서명, 5분 timestamp 범위와 Firestore nonce 재전송 차단
- Cloud relay 설정의 Windows 자격 증명 관리자 저장, 15초 로컬 폴링과 SQLite 안전 정책 재검증
- 원격 큐 등록 후 `queued` 또는 `awaiting_local_approval` 상태를 Telegram에 회신

## 안전 경계

원격 사용자는 브로커·거래소·위험 정책과 주문 함수를 직접 호출하지 않는다. 투자 또는 운영 지시를 승인해도 원격 작업 상태가 `approved`로 바뀔 뿐이며 실제 모의주문 후보는 기존 데이터 신선도·원장·결정론적 위험 게이트와 별도 사용자 승인을 통과해야 한다. 실전 주문은 계속 잠겨 있다.

Tauri IPC의 정규화 요청은 로컬 앱 경계만 의미한다. 공개 네트워크의 Cloud relay 요청은 HMAC, timestamp와 Firestore nonce replay 방지를 먼저 검증한 뒤에만 이 엔진에 전달한다. Telegram Bot token과 Cloud 자격정보는 SQLite·React·로그가 아니라 OS 보안 저장소 또는 Cloud Secret Manager에 저장한다.

Cloud relay 소스와 배포 절차는 [`server/relay/README.md`](../server/relay/README.md)에 있다. Google Cloud MFA, 전용 프로젝트, Firestore, 최소 권한 서비스 계정, Secret Manager와 Cloud Run 리비전은 적용했다. 다만 공식 `run.app` URL이 컨테이너 도달 전 HTTP 404를 반환하므로 webhook 등록과 실제 왕복은 보류했다. 세부 진단은 [Google Cloud relay 배포 상태](google-cloud-relay-deployment-status.md)에 기록한다. Gemini는 1차 원격 왕복에 포함하지 않고 로컬 Codex 처리 경로를 우선 사용한다.

## 후속 연결 순서

1. Google Cloud `run.app` 공개 URL 404를 해소하고 `/healthz` HTTP 200을 확인한다.
2. Secret Manager의 token과 webhook secret으로 Telegram webhook을 등록한다.
3. 허용 사용자의 실제 메시지 수신과 미허용·잘못된 secret 거부를 검증한다.
4. desktop HMAC poll, nonce 재전송 차단과 Firestore 임대 만료 복구를 실제 왕복으로 검증한다.
5. 소액 예산 알림과 운영 모니터링을 설정한다.
6. AI 공급자를 `local_codex` 또는 `gemini_cloud`로 연결한다.
7. 분석 결과 회신과 로컬 승인 알림을 연결한다.

Google AI 앱 구독과 Gemini API 과금은 별도다. 학생 혜택의 Cloud 크레딧을 적용하더라도 Billing 예산 알림, quota와 최대 요청 크기를 설정한다.
