# Google Cloud relay 배포 상태

기준일: 2026-08-26

## 완료

- 전용 프로젝트: `investa-remote-bumniverse`
- 결제 계정 연결 확인, 확인 당시 예상 요금 0원
- 계정 MFA 활성화
- Cloud Run Admin API
- Cloud Build API
- Artifact Registry API
- Cloud Firestore API
- Secret Manager API
- Firestore `(default)`
  - Standard 버전
  - 기본 모드
  - 기본 거부 보안 규칙
  - `asia-northeast3` 서울 리전
- `investa-relay` 전용 서비스 계정
  - 프로젝트 수준 권한은 `roles/datastore.user`만 부여
  - Secret 세 개는 Secret Manager 접근 권한으로만 분리
- Telegram Bot token, webhook secret, desktop shared secret의 Secret Manager 등록
- `relay_nonces.expiresAt` TTL 활성화
- Cloud Run 런타임 적용
  - Secret 원문 대신 Secret Manager 참조만 주입
  - 최소 인스턴스 0, 최대 인스턴스 1, 동시성 20, 256 MiB, 15초 timeout
  - `investa-relay`와 라우팅 진단용 `investa-relay-v2` 리비전이 모두 `Ready`
  - 실전 주문은 `liveOrderEnabled=false`로 유지

## 현재 차단 문제

- 두 서비스 모두 Cloud Run 상태와 traffic routing은 정상이나 공식 `run.app` URL이 Google 프런트엔드에서 HTTP 404를 반환한다.
- 동일 소스를 Cloud Shell에서 직접 실행한 `/healthz`는 HTTP 200이므로 애플리케이션 라우팅이나 시작 실패가 아니다.
- 공개 ingress, 기본 URL 활성화, invoker IAM check 해제, 새 서비스명 재배포를 확인했지만 요청 로그가 생성되지 않았다. 요청이 컨테이너보다 앞선 Google 라우팅 계층에서 중단되는 상태다.
- Google Cloud 공개 상태판과 프로젝트별 Personalized Service Health에는 서울 `asia-northeast3` Cloud Run 활성 장애가 없다.
- Cloud Run 콘솔에서 `전체 인터넷`, 기본 HTTPS 엔드포인트 `사용 설정`, `공개 액세스 허용`을 다시 확인했다.
- 최근 2일의 `run.googleapis.com/HttpIngress` 정책 거부 로그는 0건이다.
- 프로젝트는 조직에 속하지 않은 소비자 프로젝트라 프로젝트에 적용된 사용자 정의 조직 정책이나 VPC Service Controls 경계가 원인일 가능성도 낮다.
- 작동하지 않는 URL을 Telegram에 등록하면 업데이트 전달이 실패하므로 webhook은 등록하지 않았다.

## 남은 운영 적용

- Google Cloud 지원에 두 Ready 서비스의 `run.app` 호스트 매핑 또는 serving control-plane 상태 확인 요청
- 공개 `/healthz` HTTP 200 확인 후 Telegram webhook 등록
- 허용 사용자 메시지 수신 → Firestore → desktop HMAC poll → 결과 회신 실제 왕복 검증
- 소액 예산 알림 설정

Bot token과 shared secret 원문은 이 문서, Git, SQLite, React 상태와 로그에 남기지 않는다.
