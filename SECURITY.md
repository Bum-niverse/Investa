# Security Policy

## 지원 범위

Investa는 로컬 우선 연구·모의투자 프로그램입니다. 현재 배포판은 외부 실주문을 전송하지 않으며 `SHADOW ONLY` 잠금을 기본값으로 유지합니다.

GitHub 로그인은 설치된 GitHub CLI 세션의 사용자 ID와 로그인명만 확인하는 로컬 진입 게이트입니다. Investa는 GitHub access token을 요청·반환·저장하지 않습니다. 이 게이트는 로컬 SQLite 암호화나 운영체제 사용자 인증을 대신하지 않으므로 Windows 계정 잠금과 디스크 보호를 함께 사용해야 합니다.

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

## 유지관리자 검증

릴리스 전 최소 검증 항목은 다음과 같습니다.

1. `cargo test --manifest-path src-tauri/Cargo.toml`
2. `cargo audit --file src-tauri/Cargo.lock`
3. Gitleaks로 작업 디렉터리와 전체 Git 이력 검사
4. 프론트 번들과 로그에 비밀값·계좌번호가 포함되지 않았는지 검사
5. 읽기 전용 키로 정상 조회, 주문·출금 권한이 없는 키 경계를 별도 계정에서 확인
