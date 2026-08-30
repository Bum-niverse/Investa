# Investa ML worker

`investa-ml-worker-v1` 단일 매니페스트와 `investa-ml-worker-sharded-v1` XGBoost shard bundle을 입력받아 다중분류 기준 모델을 학습하고, 시간순 test 구간에서만 OOS 지표를 계산한다.

## 안전 경계

- 네트워크, 계좌, 주문, 출금 기능이 없다.
- `liveOrderAllowed`가 하나라도 `false`가 아니면 거부한다.
- Rust가 고정한 dataset·feature schema·job 입력 SHA-256을 재검증한다.
- 완료 시각 이후 정보, split 경계 누수, 중복·결측 feature를 거부한다.
- Python pickle을 만들거나 읽지 않는다. LightGBM text와 XGBoost JSON만 저장한다.
- 성공 결과도 `candidate_review` 등록용일 뿐 자동 배치나 주문 권한이 아니다.
- shard 계약은 Rust가 고정 staging한 파일명만 허용하고 경로 이탈·크기·결합/자식/스키마 해시·split 순서·중복을 재검사한다.
- XGBoost shard는 `DataIter`·`ExtMemQuantileDMatrix(hist)`로 순차 소비한다. LightGBM shard는 안전한 out-of-core 경로가 검증될 때까지 거부한다.

## 설치

Windows 예시(저장소 밖 격리 환경):

```powershell
$venv = Join-Path $env:LOCALAPPDATA 'Investa\ml-worker-venv'
python -m venv $venv
& (Join-Path $venv 'Scripts\python.exe') -m pip install --require-hashes -r ml-worker/requirements.lock
```

LightGBM 4.7.0은 MIT, XGBoost 3.4.1은 Apache-2.0 라이선스다. NumPy·SciPy는 BSD 계열이고 Narwhals는 MIT다. 앱 패키지에 재배포하기 전에는 각 패키지의 라이선스 고지 파일을 번들에 포함해야 한다.

## 실행

```powershell
& (Join-Path $env:LOCALAPPDATA 'Investa\ml-worker-venv\Scripts\python.exe') `
  ml-worker/investa_ml_worker.py --input bundle.json --output-dir .\worker-output
```

표준 출력에는 Rust `ml_training_job_complete`가 받을 수 있는 JSON만 출력한다. 모델 아티팩트와 동일 결과 JSON은 `--output-dir` 아래에 저장된다.

## 테스트

```powershell
& (Join-Path $env:LOCALAPPDATA 'Investa\ml-worker-venv\Scripts\python.exe') `
  -m unittest discover -s ml-worker/tests -v
```

입력 크기·표본/feature 수 상한과 함께 Rust runner가 Windows Job Object 메모리 상한, timeout, 출력 크기와 자식 종료를 강제한다. XGBoost 외부 메모리 캐시는 작업 출력 폴더 안에만 만들고 학습 종료 시 제거한다.
