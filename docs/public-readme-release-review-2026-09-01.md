# 공개 저장소 전환·README 제품 화면 검토

기준일: 2026-09-01

## 보안 사전 검토

공개 전환은 현재 파일뿐 아니라 전체 Git 이력, 커밋 작성자 메타데이터와 문서 이미지를 인터넷에 노출한다. 추적·추가 예정 파일과 전체 패치 이력에서 고신뢰 비밀 패턴, 금지 확장자와 로컬 절대 경로를 검사했다. 제품 캡처에는 API Key·Secret, 전체 계좌번호, 실제 보유자산 원문과 Telegram 식별자를 포함하지 않는다. 내부 모의잔고와 시스템 연결용 백테스트는 실제 계좌·수익 성과로 오인하지 않도록 README에서 명시한다.

사용자 승인에 따라 모든 원격 브랜치와 현재 Pull Request 참조의 기존 커밋 작성자 이메일을 GitHub 비공개 noreply 주소로 재작성했다. 재작성 전 전체 refs는 워크스페이스 밖 로컬 bundle로 백업했으며, 각 브랜치의 tree hash가 바뀌지 않았음을 확인한 뒤 강제 갱신했다. 저장소에는 라이선스 파일이 없어 공개하더라도 제3자 재사용 권한을 부여하는 오픈소스 배포로 간주하지 않는다.

## 레퍼런스 검토

### GitHub

GitHub 공식 문서는 README가 제품 목적·사용 이유·시작 방법·지원 경로를 설명하는 저장소의 첫 화면이며 상대 이미지 경로를 branch에 맞게 해석한다고 설명한다. 공개 저장소는 누구나 전체 코드와 이력을 볼 수 있으므로 secret scanning·push protection·보안 정책을 함께 사용하라는 보안 지침도 적용한다.

- https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/about-readmes
- https://docs.github.com/en/repositories/creating-and-managing-repositories/about-repositories
- https://docs.github.com/en/code-security/getting-started/quickstart-for-securing-your-repository

### Google 검색

이번 변경은 Google 제품·API·데이터를 도입하지 않는 GitHub 문서·로컬 화면 캡처 작업이다. 적용할 런타임 구현이나 데이터셋은 없으며, 검색 결과보다 GitHub의 저장소 표시·보안 문서를 직접 근거로 사용한다.

### Kaggle

소프트웨어 README 화면 구성이나 공개 저장소 보안 경계를 검증할 공식 Dataset·Model·Notebook은 없다. 재현 가능한 ML 산출물 공개 작업이 아니므로 Kaggle 자료는 적용하지 않는다.

## 적용

- 기존 로그인 전용 GIF를 사옥·분석 보관함·통합 모의투자·append-only 원장 순환 화면으로 교체한다.
- 각 핵심 화면의 정적 이미지를 README 갤러리에 추가한다.
- 캡처는 로컬 Tauri 개발 빌드의 실제 렌더링이며 계좌 연결이나 주문을 새로 실행하지 않는다.
- 공개 전 `scripts/security_audit.py`, 전체 이력 고신뢰 패턴 검사, 이미지 메타데이터와 README 상대 경로를 다시 확인한다.
