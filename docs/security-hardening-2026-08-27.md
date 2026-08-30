# Investa 배포 전 보안 강화 계획·구현 기록

기준일: 2026-08-27
범위: Tauri 데스크톱, Codex·GitHub CLI, 금융 API 자격정보, Telegram Cloud Relay, 로컬 데이터, 배포 산출물

## 위협 모델과 불변 경계

- Investa는 `SHADOW ONLY`이며 실제 주문·출금 함수를 제공하지 않는다.
- LLM은 분석 제안만 수행하고 계좌 자격정보, 주문 도구, 위험정책 변경 권한을 받지 않는다.
- 비밀값은 코드·문서·로그·Git·Firestore 작업 본문에 저장하지 않는다.
- GitHub 로그인은 온라인 계정 동기화가 아니라 이 Windows 로컬 작업공간의 소유자 확인 게이트다.
- 금융 계좌 연결은 읽기 전용·IP 제한이 검증된 경우에만 허용한다. 공급자 API로 검증할 수 없는 권한은 “미검증”으로 표시한다.

## 이번 보안 묶음에서 구현한 항목

### Codex 분석 경계

- Codex 하위 프로세스 환경을 allowlist로 재구성해 금융·클라우드·GitHub 토큰의 우발적 상속을 차단한다.
- 저장소가 아닌 비어 있는 임시 분석 작업공간에서 스레드를 시작한다.
- App Server의 명시적 `readOnly` sandbox와 네트워크 차단 정책을 사용한다.
- 승인 정책은 `never`이며 서버가 보내는 실행 요청도 거부한다.

### GitHub 세션 경계

- `gh.exe`의 절대 경로를 해석한 뒤 직접 실행하고 PowerShell 명령 문자열 실행을 제거한다.
- `GH_TOKEN`, `GITHUB_TOKEN` 등 환경 토큰을 하위 프로세스에서 제거한다.
- 최초로 확인한 GitHub 숫자 ID를 Windows 자격 증명 관리자에 고정하고 다른 계정의 로컬 데이터 접근을 차단한다.
- 로그인 화면은 데이터가 GitHub 계정에 저장된다는 오해 대신 Windows 로컬 작업공간 저장 사실을 표시한다.

### Telegram Cloud Relay

- Telegram 지시를 Firestore에 넣기 전에 토큰·비밀번호·개인키 패턴을 거부한다.
- 데스크톱 결과도 Telegram 전송·Firestore 저장 전에 동일하게 검사한다.
- 작업에 24시간 기본 TTL을 기록하고 만료 작업을 임대하지 않는다.
- IP rate-limit bucket을 정리·상한 처리해 메모리 남용을 막는다.
- webhook secret, 사용자 allowlist, HMAC 서명, nonce replay 차단과 로컬 재승인 경계를 유지한다.

### 금융 자격정보

- Binance는 저장 전에 공식 API 권한 응답으로 읽기 허용, IP 제한, 거래·출금·이체 권한 비활성화를 검사한다.
- Upbit는 권한 목록을 조회할 공식 API가 없으므로 조회 성공과 권한 검증을 분리하고 사용자 확인 필요 상태를 표시한다.
- 키 원문은 Windows 자격 증명 관리자 밖에 저장하지 않는다.

### CSP와 릴리스 검사

- 프로덕션 CSP에서 개발 서버의 localhost·WebSocket 출처를 제거하고 개발 CSP로 분리한다.
- object·base·frame 삽입을 차단한다.
- `scripts/security_audit.py`가 Git 추적 파일의 비밀 패턴과 환경·DB·인증서 추적 여부를 검사한다.

### 의존성 감사 결과

- `cargo audit --file src-tauri/Cargo.lock`은 알려진 취약점으로 빌드를 실패시키지 않았지만 18개의 허용 경고를 보고했다.
- 경고에는 Linux GTK3 전이 의존성의 유지중단 항목과 `event-listener`, `glib`의 unsound 항목이 포함된다.
- 직접 의존성만 임의로 올리면 Tauri 2 플랫폼 호환성이 깨질 수 있으므로, Tauri·플러그인 호환표를 확인한 별도 업그레이드 묶음과 Windows/macOS/Linux 빌드 검증 전까지 공개 배포 잔여 위험으로 유지한다.
- Gitleaks 실행 파일은 현재 PC에 없어 자체 Git 전체 이력 스캔으로 보완했다. 정식 릴리스 CI에는 Gitleaks를 별도 적용해야 한다.

### 로컬 데이터와 소셜 로그인

- `%APPDATA%\\com.bumniverse.investa`에 `CodexSandboxUsers` 명시적 거부 ACL을 적용하고 앱 시작 시 동일 경계를 재확인한다.
- Google 선택 로그인은 데스크톱 OAuth PKCE·state·127.0.0.1 임시 callback을 사용하며 access/refresh token을 저장하지 않는다.
- GitHub와 Google은 각각 최초 불변 사용자 ID를 Windows 자격 증명 관리자에 고정한다.
- Apple 로그인은 HTTPS callback과 Developer 설정 전에는 버튼과 명령 경로를 활성화하지 않는다. Apple 앱 배포·notarization은 이번 범위에서 제외한다.
- GitHub Actions에 Gitleaks, 프론트·Rust·Relay 검증과 RustSec 감사를 추가하고 Dependabot이 npm·Cargo·Actions 업데이트를 제안하도록 설정했다.

### Google Cloud 읽기 전용 운영 감사

- 두 Cloud Run relay가 공개·전체 ingress 상태임을 확인했다. Telegram webhook 특성상 공개 endpoint는 필요하지만 secret/HMAC 경계가 실제 왕복되기 전까지 운영 준비 완료로 보지 않는다.
- 전용 relay 서비스 계정은 Datastore User만 보유한다.
- 기본 compute 서비스 계정의 프로젝트 Editor 역할은 잔여 과권한 후보이며 사용 여부 확인 후 축소해야 한다.
- Firestore `relay_nonces.expiresAt` TTL은 활성이나 `relay_jobs.expiresAt` TTL은 없다. 후자는 만료 문서를 실제 삭제하므로 별도 운영 확인 뒤 생성한다.
- 공식 Cloud Run `/healthz`는 여전히 Google 프런트 404라 Telegram webhook 등록을 계속 차단한다.

## 외부 권한 또는 운영 적용이 남은 항목

- 기존에 노출되었을 가능성이 있는 Telegram·Cloud·금융 키의 폐기·재발급 여부 확인
- Google Cloud Run 비공개 전환 또는 인증 프록시 적용, 최소 IAM과 Firestore TTL 정책의 실제 배포 검증
- Cloud 로그의 사용자 ID·지시 본문 보존/마스킹 정책과 예산·rate-limit 알림 검증
- Windows 코드 서명과 자동 업데이트 서명 키. macOS Developer ID·notarization은 사용자 검증 이후 별도 단계로 연기했다.
- GitHub Actions의 build provenance·서명 게이트. Gitleaks·테스트·RustSec 감사 워크플로는 구현했으나 원격 Actions 왕복은 아직 미검증이다.
- Tauri 호환 범위 안에서 18개 RustSec 유지중단·unsound 경고를 해소하거나 플랫폼별 비영향 근거를 기록
- 로컬 SQLite 자체 암호화 여부 결정. 현재는 Windows 계정 경계에 더해 Codex sandbox 명시적 거부 ACL을 적용했지만 DB 암호화는 완료로 간주하지 않는다.
- Upbit 관리 화면에서 주문·출금 권한 비활성화를 사용자가 확인하는 절차

## 배포 차단 조건

다음 중 하나라도 충족하면 공개 배포하지 않는다.

1. 비밀 스캔 또는 전체 테스트·빌드가 실패한다.
2. 실제 주문·출금 경계가 활성화되거나 AI에 실행 권한이 노출된다.
3. Cloud webhook 서명·allowlist·replay·TTL이 운영 환경에서 검증되지 않는다.
4. Binance 읽기 전용·IP 제한 검사가 실패한다.
5. 설치 파일 코드 서명과 업데이트 무결성 정책이 정해지지 않았다.

## 검증 명령

```powershell
python scripts/security_audit.py
pnpm test:frontend
pnpm build
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
Push-Location server/relay; node --test; Pop-Location
```

운영 Cloud/IAM/TTL과 실제 공급자 권한은 코드 테스트와 별개로 검증 결과를 남긴다.
