# Google Cloud relay 배포 상태

기준일: 2026-08-27

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
  - 현재 저장소의 relay 소스를 Cloud Shell에 다시 업로드해 `node --test` 6개 통과 확인
  - 현재 소스로 `investa-relay-00003-v7h` 재배포, 최신 리비전 100% 트래픽 적용
  - 실전 주문은 `liveOrderEnabled=false`로 유지

## 현재 차단 문제

- 두 서비스 모두 Cloud Run 상태와 traffic routing은 정상이나 공식 `run.app` URL이 Google 프런트엔드에서 HTTP 404를 반환한다.
- 동일 소스를 Cloud Shell에서 직접 실행한 `/healthz`는 HTTP 200이므로 애플리케이션 라우팅이나 시작 실패가 아니다.
- 공개 ingress, 기본 URL 활성화, invoker IAM check 해제, 새 서비스명 재배포를 확인했지만 요청 로그가 생성되지 않았다. 요청이 컨테이너보다 앞선 Google 라우팅 계층에서 중단되는 상태다.
- Google Cloud 공개 상태판과 프로젝트별 Personalized Service Health에는 서울 `asia-northeast3` Cloud Run 활성 장애가 없다.
- Cloud Run 콘솔에서 `전체 인터넷`, 기본 HTTPS 엔드포인트 `사용 설정`, `공개 액세스 허용`을 다시 확인했다.
- 2026-08-27 재검사에서 `ingress=all`, 기본 URL 비활성화 annotation 없음, `allUsers`의 `roles/run.invoker`를 확인했다.
- `webhook` traffic tag를 제거하고 최신 리비전 100% 라우팅을 다시 생성했지만 공식 URL과 상태 API가 반환한 URL 모두 같은 Google 404를 반환했다.
- 재배포 뒤 최신 리비전 로그에는 정상 startup/TCP probe만 있고 `/healthz` HTTP 요청 로그가 생성되지 않았다. 요청이 새 컨테이너에 도달하지 않는다는 증거다.
- 최근 2일의 `run.googleapis.com/HttpIngress` 정책 거부 로그는 0건이다.
- 프로젝트는 조직에 속하지 않은 소비자 프로젝트라 프로젝트에 적용된 사용자 정의 조직 정책이나 VPC Service Controls 경계가 원인일 가능성도 낮다.
- 작동하지 않는 URL을 Telegram에 등록하면 업데이트 전달이 실패하므로 webhook은 등록하지 않았다.

## 2026-08-27 보안 재감사

- Cloud Run 서비스 목록에서 `investa-relay`와 `investa-relay-v2`가 모두 서울 리전, `Public access`, ingress `All`로 확인됐다. Telegram은 Google IAM 토큰을 보낼 수 없으므로 공개 endpoint 자체는 의도된 경계이며 webhook secret과 desktop HMAC 검증이 필수다.
- `investa-relay` 보안 탭과 YAML에서 공개 액세스, ingress `all`, invoker IAM 검사 비활성 상태를 확인했다. 최대 인스턴스는 콘솔 서비스 수준에서 1개로 표시된다.
- 공식 `investa-relay` URL의 `/healthz`는 같은 날 다시 확인해도 Google 프런트 404이며 컨테이너 health 응답에 도달하지 않았다.
- 프로젝트 IAM에서 전용 `investa-relay` 서비스 계정은 `Cloud Datastore 사용자`만 보유한다.
- 기본 compute 서비스 계정에는 프로젝트 `편집자`가 남아 있다. relay가 전용 서비스 계정을 사용한다는 기존 배포 증거와 별개로, 사용하지 않는 계정이라면 Editor 제거 또는 계정 비활성화를 검토해야 한다.
- Firestore TTL은 `relay_nonces.expiresAt`만 `제공 중`이다. `relay_jobs.expiresAt` TTL은 아직 없으므로 코드가 기록한 만료 시각만으로 문서가 자동 삭제되지 않는다.
- `relay_jobs` TTL 생성은 기존 만료 문서의 실제 삭제를 예약하는 운영 변경이므로 별도 실행 확인 후 적용한다.
- 값을 출력하지 않는 읽기 전용 감사 스크립트는 `scripts/audit_google_cloud_security.ps1`에 있다. 현재 로컬 PC에는 `gcloud`가 없어 Cloud Shell에서 실행해야 한다.

## 남은 운영 적용

- Google Cloud 지원에 두 Ready 서비스의 `run.app` 호스트 매핑 또는 serving control-plane 상태 확인 요청
- 공개 `/healthz` HTTP 200 확인 후 Telegram webhook 등록
- 허용 사용자 메시지 수신 → Firestore → desktop HMAC poll → 결과 회신 실제 왕복 검증
- 소액 예산 알림 설정
- `relay_jobs.expiresAt` TTL 정책 생성 후 상태가 `제공 중`인지 확인
- `investa-relay-v2`가 실제 webhook·클라이언트에서 사용되지 않는지 확인한 뒤 불필요하면 삭제
- 기본 compute 서비스 계정의 Editor 역할이 필요한 워크로드가 있는지 확인하고 불필요하면 최소권한으로 축소

Bot token과 shared secret 원문은 이 문서, Git, SQLite, React 상태와 로그에 남기지 않는다.
