# macOS 호환성 기반

## 현재 범위

Investa의 제품 기준은 아직 Windows 개발 빌드다. 이번 변경은 macOS 배포를 선언하는 작업이 아니라, 동일한 Tauri 2·React·Rust 소스가 macOS에서 안전하게 검증될 수 있도록 운영체제 경계를 정리한 것이다.

- `keyring`의 `v1` 기능으로 macOS Keychain과 Windows 자격 증명 관리자를 같은 인터페이스에서 사용한다.
- Unix 앱 데이터 디렉터리는 생성 직후 `0700`으로 제한한다.
- 내부 섀도우 내구 검사는 Windows working set과 macOS Mach resident memory를 운영체제별로 읽는다.
- `.icns` 아이콘과 Tauri 번들 설정은 유지한다.
- GitHub Actions의 `macos-compatibility-manual`은 자동 실행하지 않는다. private 저장소의 macOS runner 비용을 사용자가 명시적으로 시작한 경우에만 소비한다.

## 의도적으로 막힌 기능

- ML worker의 프로세스 메모리 상한과 자식 프로세스 일괄 종료는 Windows Job Object 구현만 검증됐다. macOS에서는 동일한 fail-closed 격리를 구현하기 전까지 worker 실행을 거부한다.
- Apple 로그인은 Developer Services ID, 검증 도메인, HTTPS callback과 서버 토큰 검증 전까지 비활성이다.
- 실제 macOS 코드 서명, notarization, Gatekeeper 설치 시험은 아직 수행하지 않았다.
- 금융 자격정보·주문 권한·실주문은 운영체제와 관계없이 `SHADOW ONLY` 경계를 벗어나지 않는다.

## 수동 검증 순서

1. GitHub Actions에서 `macos-compatibility-manual`을 수동 실행한다.
2. frontend test/build, Rust format/check/test가 모두 통과했는지 확인한다.
3. 별도 시험 Mac에서 Keychain 저장·삭제, 앱 데이터 `0700`, OAuth loopback callback과 중단 복구를 확인한다.
4. Developer ID Application 인증서와 notarization 자격정보는 GitHub Environment 보호·Secret에만 저장한다.
5. 서명 후 `codesign --verify --deep --strict`, `spctl --assess`, notarization 결과를 릴리스 증거에 첨부한다.

## 채택 근거

- Apple Keychain Services: https://developer.apple.com/documentation/security/keychain-services
- Apple notarization: https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution
- Tauri macOS signing: https://v2.tauri.app/distribute/sign/macos/
- keyring-rs: https://github.com/open-source-cooperative/keyring-rs

Kaggle에는 데스크톱 앱의 Keychain, Mach resident memory, 코드 서명과 직접 관련되며 운영 검증에 사용할 수 있는 데이터셋이 없어 채택하지 않았다.
