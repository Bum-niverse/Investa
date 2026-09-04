# Investa 데스크톱 런처

기준일: 2026-09-01

## 문제와 원인

기존 바탕화면 바로가기는 `src-tauri/target/release/investa.exe`를 직접 실행했다. 이 파일은 소스 변경 뒤 자동으로 갱신되지 않는다. 최신 개발 빌드가 SQLite 스키마와 작업공간 상태를 갱신한 뒤 오래된 release를 다시 열면 초기화 단계에서 Rust panic으로 종료될 수 있으며, Windows GUI release는 콘솔이 없어 사용자가 원인을 볼 수 없다.

## 동작

`scripts/launch_investa.ps1`은 다음 순서로 동작한다.

1. React·Rust·Tauri 설정·ML worker와 잠금 파일 중 가장 최근 수정 시각을 확인한다.
2. release가 없거나 입력보다 오래된 경우에만 기존 `pnpm tauri build --no-bundle`을 실행한다.
3. 빌드가 성공한 뒤 release를 실행한다.
4. 같은 release 창이 이미 열려 있으면 새 프로세스를 만들지 않고 기존 창을 활성화한다.
5. 빌드 실패는 숨기지 않고 `%LOCALAPPDATA%\Investa\launcher\launcher.log`와 오류 대화상자에 남긴다.
6. `pnpm.cmd`와 `node.exe`를 각각 확인하고, 일반 Node 설치 또는 Codex 번들 런타임이 있으면 현재 런처 프로세스에만 안전하게 연결한다.
7. pnpm의 표준 출력과 표준 오류는 진단 로그에 보존하되, 빌드 성공 여부는 출력 채널이 아니라 실제 프로세스 종료 코드로 판단한다.
8. 절대 경로로 찾은 `pnpm.cmd`의 디렉터리를 현재 런처 PATH에 추가한다. 따라서 Tauri의 중첩 `beforeBuildCommand`도 같은 pnpm을 찾아 바탕화면 바로가기 환경에서 재빌드할 수 있다.

저장소에서는 `pnpm desktop:start`로 같은 런처를 실행하고 `pnpm desktop:check`로 빌드 필요 여부만 확인한다. release는 Tauri의 `frontendDist`를 내장하므로 localhost에 접속하지 않는다. `devUrl=http://localhost:1430`은 `pnpm tauri dev`에서만 사용한다.

`-CheckOnly`는 빌드나 실행 없이 판정 JSON만 출력한다. `runtimeReady`, `nodePath`, `pnpmPath`, `runtimeError`로 숨겨진 바로가기 환경에서도 재빌드 런타임을 실제로 찾았는지 확인할 수 있다.

## 보안 경계

- 외부 URL, 사용자 입력, 환경변수 명령 문자열을 실행하지 않는다.
- 저장소 내부의 고정된 빌드 입력과 기존 pnpm만 사용한다.
- 자격정보, 계좌정보와 환경변수 값은 로그에 기록하지 않는다.
- 자동 업데이트나 인터넷 다운로드를 수행하지 않는다.
- 실행 파일 서명·배포 검증을 대신하지 않는다.

## 레퍼런스 결정

- Tauri v2 공식 배포 문서와 upstream CLI가 지원하는 `build --no-bundle`을 채택했다. 설치 패키지를 매번 다시 만들지 않고 현재 Windows release만 갱신한다.
- Microsoft PowerShell 공식 `Start-Process`의 `-WorkingDirectory` 계약을 사용한다.
- `Start-Process`가 현재 프로세스의 환경변수를 상속하는 공식 계약에 따라 검증된 pnpm 디렉터리만 PATH에 추가하고, 임의 명령 문자열이나 외부 다운로드는 사용하지 않는다.
- GitHub 공개 구현은 별도 런처 라이브러리를 추가할 필요가 없어 채택하지 않았다.
- Kaggle은 데스크톱 실행·빌드 문제와 관련된 데이터셋이나 모델 근거가 없어 적용 가능한 결과가 없다.

참고:

- https://v2.tauri.app/distribute/
- https://github.com/tauri-apps/tauri-docs/blob/v2/src/content/docs/ja/distribute/index.mdx
- https://learn.microsoft.com/powershell/module/microsoft.powershell.management/start-process
