# Google·Apple 로그인 보안 설계

기준일: 2026-08-27

## 제품 경계

- 로그인은 로컬 작업공간 진입을 구분하는 게이트이며 SQLite 암호화나 Windows 사용자 인증을 대신하지 않는다.
- 첫 번째로 검증된 GitHub 또는 Google 계정 하나가 로컬 작업공간의 주 소유자가 된다.
- 이후 공급자 계정은 소유자로 로그인한 세션에서 명시적으로 연결한 경우에만 같은 작업공간을 열 수 있다.
- GitHub CLI 로그인을 기본값으로 유지한다.
- Google은 선택 로그인으로 제공한다. OAuth access token은 사용자 확인이 끝날 때까지만 메모리에 두며 refresh token을 요청하거나 저장하지 않는다.
- Apple 앱 배포, Developer ID, notarization은 사용자 검증 이후로 미룬다.
- Apple ID 로그인은 Apple Developer Services ID, 검증된 도메인, HTTPS callback과 서버측 토큰 검증이 갖춰지기 전에는 활성화하지 않는다.
- 어떤 로그인 공급자도 금융 API 자격정보, 주문 권한, 위험정책 변경 권한을 받지 않는다.

## Google 데스크톱 OAuth

1. 사용자가 Google Cloud에서 **Desktop app** 유형 OAuth Client ID를 발급한다.
2. Client ID와 Google이 발급한 Desktop Client Secret은 설정 화면을 통해 Windows 자격 증명 관리자에만 저장한다. 소스·로그·SQLite·ProjectStudio에는 기록하지 않는다.
3. 로그인 때마다 PKCE verifier/challenge와 state를 새로 만든다.
4. 시스템 브라우저를 열고 `127.0.0.1`의 임시 포트에서 한 번만 callback을 받는다.
5. state 일치와 이메일 검증 상태를 확인한다.
6. Google의 불변 `sub`를 통합 작업공간 소유자 레코드와 대조한다. 첫 로그인이라면 주 소유자로 만들고, 기존 작업공간이라면 미리 연결된 `sub`만 허용한다.
7. 브라우저 하위 프로세스에는 금융·Cloud·GitHub 환경변수를 상속하지 않는다.
8. 임시 loopback callback은 최대 10분만 유지한다. 만료된 `127.0.0.1` 주소는 재사용하지 않으며 사용자는 앱에서 Google 로그인 버튼을 다시 눌러 새 PKCE·state·포트를 발급받는다.

요청 scope는 `openid email profile`로 제한하고 `access_type=online`을 사용한다. Google 인증 플랫폼의 데이터 액세스에도 같은 최소 범위만 등록한다.

## 통합 작업공간 소유권과 계정 연결

- 소유자 레코드는 임의 UUID, 주 공급자와 연결된 공급자별 불변 사용자 ID만 포함하며 Windows 자격 증명 관리자에 저장한다.
- 이메일·표시 이름은 소유권 판정에 사용하거나 영구 저장하지 않는다.
- 연결되지 않은 계정은 OAuth 또는 GitHub 검증에 성공해도 SQLite에 진입하지 못한다.
- 새 계정 연결은 이미 소유자로 인증된 현재 앱 세션에서만 가능하다. 로그인 화면의 실패를 계정 연결로 자동 승격하지 않는다.
- 기존 버전에서 GitHub와 Google 소유자 항목이 모두 발견되면 자동 병합하지 않고 기존 기본 게이트인 GitHub만 주 소유자로 승계한다. Google은 GitHub 소유자로 로그인한 뒤 설정에서 다시 연결한다.
- 공급자 access/refresh token과 GitHub CLI token은 소유자 레코드에 포함하지 않는다.

## 연결 해제·세션 만료·복구·탈퇴

- 앱 메모리의 인증 세션은 명시적 로그아웃 또는 앱 종료 때 폐기하며 공급자 access token을 세션 복구 수단으로 저장하지 않는다.
- 연결 해제는 인증된 소유자 세션에서만 가능하고 공급자 이름별 확인 문구를 다시 요구한다. 주 소유자 공급자와 마지막 소유자 계정은 해제할 수 없고 다른 계정으로 자동 이전하지 않는다.
- 연결 계정을 해제해도 SQLite 분석·백테스트·모의원장, 차트 선, 감사 기록과 검증 백업은 유지한다. 이는 로그인 수단 제거이지 투자 기록 삭제가 아니다.
- 소유자 복구는 이미 연결된 GitHub·Google·향후 Apple 불변 사용자 ID 중 하나의 정상 재인증만 허용한다. 이메일·표시 이름·지원자 판단·로컬 파일 소유만으로 우회 복구하지 않는다.
- 작업공간 전체 삭제 시에는 소유자 재인증, 최신 검증 백업 선택, SQLite·차트 로컬 저장·Windows 자격 증명 관리자 항목·로컬 백업·내보낸 감사 파일의 삭제 범위를 별도로 확인해야 한다. 이 원자적 삭제·실패 롤백 흐름은 아직 구현되지 않았으므로 UI와 API에서 비활성화한다.

## Apple ID 로그인

Apple authorization callback은 localhost나 IP 주소를 허용하지 않는다. 따라서 Cloud relay를 임의의 공개 callback으로 재사용하지 않고 다음 조건을 모두 만족한 별도 인증 경계를 만든 뒤 활성화한다.

- Apple Developer Services ID와 Sign in with Apple 설정
- 소유권이 확인된 HTTPS 도메인과 정확한 redirect URI allowlist
- state·nonce 검증, authorization code의 단일 사용과 짧은 만료
- Apple 공개키/JWKS를 이용한 ID token 서명·issuer·audience·expiry 검증
- client secret private key는 Google Secret Manager에만 저장하고 데스크톱·Firestore·로그에 저장하지 않음
- callback 결과를 데스크톱에 전달할 때 일회용 교환 코드, TTL, replay 방지와 로컬 소유자 고정 적용

위 조건을 갖추기 전에는 UI가 `Developer 설정 필요`로 표시되고 로그인 작업을 시작하지 않는다.

## 남은 실제 검증

- Google Desktop OAuth Client ID·Secret 입력 뒤 계정 선택 → callback → 앱 진입 왕복: 2026-08-27 확인
- 기존 GitHub 소유자 작업공간에서 미연결 Google 거부 → GitHub 로그인 → 설정에서 Google 연결 → Google 재로그인 왕복
- 10분 callback 만료 후 이전 주소의 연결 거부와 앱 내 재시도 안내
- 취소, state 변조, 검증되지 않은 이메일, 다른 Google 계정의 접근 거부
- Windows 재시작 후 Client ID와 소유자 고정 상태 유지
- Apple Developer 설정과 HTTPS callback은 Apple 배포 검토 시점에 별도 구현·검증

공식 근거:

- [Google OAuth 2.0 for Mobile & Desktop Apps](https://developers.google.com/identity/protocols/oauth2/native-app)
- [Request an authorization to the Sign in with Apple server](https://developer.apple.com/documentation/signinwithapplerestapi/request-an-authorization-to-the-sign-in-with-apple-server)
