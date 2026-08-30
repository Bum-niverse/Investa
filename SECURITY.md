# Security Policy

배포 전 상세 위협 모델, 구현 상태와 운영 차단 조건은 [`docs/security-hardening-2026-08-27.md`](docs/security-hardening-2026-08-27.md)를 함께 따릅니다.

## 지원 버전과 서명 상태

| 대상 | 상태 | 보안 업데이트 |
| --- | --- | --- |
| `main` 최신 소스 | 개발 중 | 적용 |
| Windows 개발 빌드 | 검증 중 | 적용 |
| macOS 호환성 기반 | 수동 CI 검증 전 | 실배포 미지원 |
| 과거 커밋·비공식 빌드 | 미지원 | 미적용 |

현재 GitHub 소스와 로컬 개발 산출물은 상용 배포용 코드 서명·notarization을 완료한 릴리스가 아닙니다. 서명되지 않은 산출물을 공식 설치 파일로 표현하지 않으며, 서명 키·인증서를 저장소나 CI 로그에 넣지 않습니다. 릴리스 후보의 실제 보안 검증 상태는 [`docs/security-release-review-2026-08-31.md`](docs/security-release-review-2026-08-31.md)에 코드 검증, 운영 적용, 미검증 항목을 분리해 기록합니다.

## 변경 전 보안 게이트

모든 코드·데이터·문서·UI·기획 변경은 구현 전에 보안 영향을 검토합니다. 보호 자산과 신뢰 경계, 인증·인가·소유권, 외부 입력, 비밀정보와 로그, 파일·DB, 네트워크·외부 API, 비용 남용, 데이터 누수, 장애·재시작과 롤백을 확인합니다. 영향이 작아도 `보안 영향 없음`을 추측으로 처리하지 않고 변경 경로를 근거로 기록합니다.

치명적 노출, 권한 우회, 비밀정보 유출 또는 비가역 데이터 손상 가능성이 발견되면 신규 기능 개발을 중지하고 노출 차단과 자격정보 폐기·교체 필요성을 우선합니다. 실제 운영 데이터나 제3자 계정을 공격하는 방식으로 검증하지 않습니다. 상세 순서와 GitHub·Kaggle·Google 사전 조사 기록은 [`docs/development-reference-policy.md`](docs/development-reference-policy.md)를 따릅니다.

## 지원 범위

Investa는 로컬 우선 연구·모의투자 프로그램입니다. 현재 배포판은 외부 실주문을 전송하지 않으며 `SHADOW ONLY` 잠금을 기본값으로 유지합니다.

GitHub 로그인은 설치된 GitHub CLI 세션의 사용자 ID와 로그인명만 확인하는 로컬 진입 게이트입니다. Investa는 GitHub access token을 요청·반환·저장하지 않습니다. 이 게이트는 로컬 SQLite 암호화나 운영체제 사용자 인증을 대신하지 않으므로 Windows 계정 잠금과 디스크 보호를 함께 사용해야 합니다.

첫 번째로 검증된 GitHub 또는 Google 계정이 통합 로컬 작업공간의 주 소유자가 됩니다. 이후 계정은 소유자로 인증된 앱 세션에서 명시적으로 연결해야 같은 SQLite를 열 수 있습니다. Google 선택 로그인은 데스크톱 OAuth PKCE와 `127.0.0.1` 임시 callback을 사용하고 access token과 refresh token을 저장하지 않으며, 소유권은 공급자의 불변 사용자 ID만 Windows 자격 증명 관리자에 보존해 판정합니다. Apple 로그인은 Services ID·검증 도메인·HTTPS callback·서버측 토큰 검증 전에는 활성화하지 않습니다. 상세 계약은 [`docs/social-login-security.md`](docs/social-login-security.md)를 따릅니다.

## 비밀정보

- API Key, Secret, token과 전체 계좌번호를 Issue, Discussion, 로그 또는 커밋에 올리지 마세요.
- 설정 화면에서 입력한 공급자 자격정보는 연결 확인 후 Windows 자격 증명 관리자에 저장됩니다.
- Upbit 키는 자산조회 권한만 허용하고 주문·출금 권한은 부여하지 마세요.
- Binance 키는 읽기 권한과 IP 제한만 허용하고 현물 거래·선물 거래·출금 권한은 활성화하지 마세요.
- Telegram Bot token, Cloud relay secret과 Gemini API key는 SQLite·React·로그·ProjectStudio 문서에 저장하지 마세요. 로컬에서는 Windows 자격 증명 관리자, Cloud에서는 Secret Manager를 사용하세요.
- Telegram 원격운영 allowlist에는 본인의 numeric user ID만 등록하세요. 투자·자동매매·시스템 제어는 원격 메시지만으로 실행되지 않고 로컬 승인과 기존 위험 게이트를 모두 통과해야 합니다.
- 키 노출이 의심되면 저장소 파일만 수정하지 말고 공급자에서 즉시 폐기·재발급하세요.

## 취약점 제보

공개 Issue에는 공격 재현에 필요한 비밀값이나 실제 계좌정보를 남기지 마세요. 저장소 소유자가 공개한 비공개 보안 연락 수단으로 최소 재현 절차, 영향 범위와 영향을 받는 버전을 전달하세요. 비공개 연락 수단이 아직 게시되지 않았다면 실제 계좌나 운영 API를 대상으로 검증하지 말고 공개 Issue에는 연락 방법 요청만 남겨 주세요.

현재 저장소의 GitHub Private Vulnerability Reporting 활성 상태는 확인되지 않았습니다. 해당 기능을 켜기 전에는 공개 Issue에 민감한 재현 정보를 게시하지 마세요.

## 유지관리자 검증

릴리스 전 최소 검증 항목은 다음과 같습니다.

1. `cargo test --manifest-path src-tauri/Cargo.toml`
2. `cargo audit --file src-tauri/Cargo.lock`
3. Gitleaks로 작업 디렉터리와 전체 Git 이력 검사
4. 프론트 번들과 로그에 비밀값·계좌번호가 포함되지 않았는지 검사
5. 읽기 전용 키로 정상 조회, 주문·출금 권한이 없는 키 경계를 별도 계정에서 확인
6. `python scripts/security_audit.py`로 Git 추적 파일의 고신뢰 비밀 패턴과 `.env`·DB·개인키·인증서 산출물 추적 여부 검사
7. Telegram Relay 지시·결과의 저장 전 비밀 차단, TTL, webhook 서명, nonce replay와 allowlist 검사
8. Codex·GitHub CLI 하위 프로세스의 금융·Cloud·GitHub 환경 토큰 비상속 확인
9. 금융 API 권한 검증 가능 여부와 “연결됨/권한 미검증” 상태 분리 확인
10. Google OAuth의 PKCE·state, 통합 소유자 고정, 미연결 계정 거부·명시적 연결과 브라우저 하위 프로세스 비밀 환경변수 비상속 확인
11. Apple 버튼이 Developer 설정 전 비활성이고 localhost·검증 없는 callback 우회가 없는지 확인
