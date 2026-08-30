"""자동매매·모델 고정 로드맵을 ProjectStudio 기능명세에 멱등 반영한다."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path
from types import ModuleType
from typing import Any


PROJECT_ID = "36e87491-74a8-48ca-a7b8-30fa6ccea131"
SECTION_MARKER = "자동매매·모델 고정 개발 로드맵 (2026-08-27)"
OLD_AGGREGATION_NOTE = "- 2026-08-28 공통 Tick·완료 봉 Rust 집계 코어를 추가했다. 1분봉과 3·5·15·30·60·240분 봉, partial 분리, gap 보존, 중복·역순·overflow 거부를 검증했으며 공식 WebSocket 연결은 미완료로 유지한다."
AGGREGATION_NOTE = "- 2026-08-28 공통 Tick·완료 봉 Rust 집계 코어를 추가했다. 1분봉과 3·5·15·30·60·240분 봉, partial 분리, gap 보존, 중복·역순·overflow 거부를 검증했다."
PUBLIC_STREAM_NOTE = "- 2026-08-28 Upbit·Binance 공개 체결 WebSocket을 공통 Tick으로 연결하고 partial·완료 1분봉·gap·마지막 순번을 SQLite에 체크포인트해 재시작 뒤 복원한다. Binance 선물 mark/index/funding은 표시 근거로 분리한다."
OLD_PUBLIC_STREAM_PENDING_NOTE = "- 2026-08-28 Upbit·Binance 공개 체결 WebSocket을 공통 Tick으로 연결하고 partial·완료 1분봉·gap·마지막 순번을 SQLite에 체크포인트해 재시작 뒤 복원한다. Binance 선물 mark/index/funding은 표시 근거로 분리하며 토스 인증 WebSocket과 REST gap 백필은 미완료다."
REST_GAP_BACKFILL_NOTE = "- 2026-08-30 Upbit·Binance 공식 공개 REST로 체크포인트의 첫 gap을 공급자별 상한 안에서 복구한다. 조회 중 상태 변경·범위 밖·미완료·중복·역순·단위 불일치는 거부하고 Upbit 무거래 분은 보간하지 않으며 liveOrderAllowed=false를 유지한다. 토스 인증 WebSocket과 24시간 실제 왕복은 미완료다."
TOSS_WEBSOCKET_NOTE = "- 2026-08-31 토스 인증 WebSocket Rust 전송을 추가했다. Authorization 토큰은 Rust handshake에서만 사용하고 React·상태·로그에 노출하지 않는다. 체결·호가 market topic만 허용하고 개인 주문 topic은 구조적으로 차단하며 60초 PING, ack timeout, 지수 backoff+jitter, 종료·재구독과 완료 봉 집계를 연결했다. 저장 자격정보로 공식 handshake·국장 구독 ack는 통과했으나 국장·미장 장중 체결·호가와 24시간 실제 내구 검증은 남아 있다."
SHADOW_SOAK_HARNESS_NOTE = "- 2026-08-31 내부 섀도우 실제 시간 수집 경로를 운영 패널에 연결했다. Windows 현재 프로세스 working set, SQLite 크기, 활성 섀도우 작업자, 내부 후보, SQLite·KRW·USD 원장 건강과 재시작 대사를 1분 표본으로 수집한다. 3분 초과 표본 공백과 재시작 대사 실패는 fail-closed이며 구현 완료가 실제 24시간 통과를 뜻하지 않는다."
STRATEGY_PLUGIN_NOTE = "- 2026-08-28 이동평균 교차·가격 채널 돌파·평균 이격 회귀·ATR 변동성 확장을 v1 순수 Rust 플러그인으로 구현했다. 백테스트와 섀도우 최신 신호가 같은 디스패처를 사용하며 혼합 플러그인·미지원 주기·필드 누락을 사전 거부한다."
CADENCE_CONTRACT_NOTE = "- 2026-08-30 tick·1m·3m·5m·15m·30m·1h·4h·1d 판단 주기와 tick·15~86,400초 실행 관리 주기를 닫힌 Rust 계약으로 분리했다. 완료 봉 플러그인의 tick 판단, 미검증 공급자, 백테스트·런타임 interval 불일치를 거부하고 성공 결과도 liveOrderAllowed=false로 유지한다."
EXECUTION_ALGORITHM_NOTE = "- 2026-08-28 internal-execution-v1을 구현했다. 최소 수량 단위 기반 분할, 최초 기준가 대비 최대 슬리피지, 재호가 횟수, 명시적 부분체결, 취소·만료와 멱등 사건을 SQLite에 보존한다. 증권 선물·코인 무기한선물은 최대 2배 격리증거금·청산 완충·reduce-only 포지션 감축을 통과해야 하며 외부 주문 전송은 없다."
STRATEGY_DEPLOYMENT_NOTE = "- 2026-08-28 strategy-deployment-v1을 구현했다. 저장된 OOS·Walk-forward 전 항목과 1.5배·2배 비용 스트레스를 다시 검증하고 experiment·dataset·전략 스키마·플러그인 버전을 SHA-256 근거로 고정한다. 명시적 승인 뒤 SHADOW Canary, 관측 기반 자동 중지, 별도 승인형 내부 모의운용과 직전 버전 롤백을 SQLite 사건으로 보존하며 외부 주문 전송은 없다."
OLD_ML_WORKER_NOTE = "- 2026-08-28 investa-ml-worker-v1 기반을 구현했다. PIT 품질 감사를 통과한 데이터 payload·피처 스키마와 학습 code·seed·horizon·파라미터를 SHA-256으로 고정하고 split 타깃 누수·변조를 거부한다. LightGBM·XGBoost·Chronos·TimesFM의 제한된 worker 계약과 실패 이력, 허용 아티팩트·OOS 지표 기반 candidate_review 등록까지만 제공하며 실제 학습 라이브러리와 자동 배치는 미완료다."
ML_WORKER_NOTE = "- 2026-08-28 investa-ml-worker-v1 기반을 구현했다. PIT 품질 감사를 통과한 데이터 payload·피처 스키마와 학습 code·seed·horizon·파라미터를 SHA-256으로 고정하고 split 타깃 누수·변조를 거부한다. 성공 결과는 허용 아티팩트·OOS 지표 기반 candidate_review로만 등록한다."
OLD_ML_BASELINE_NOTE = "- 2026-08-28 저장소 밖 Python 3.14 venv에 LightGBM 4.7.0·XGBoost 3.4.1 기준 worker를 구현했다. 동일한 시간순 train·validation·test와 synthetic PIT 데이터로 text·JSON 아티팩트의 실제 학습·CLI 왕복을 검증했으며 자동 배치·외부 주문 권한은 없다. 수년치 공식 PIT 데이터와 Rust OOS 재계산은 미완료다."
ML_BASELINE_NOTE = "- 2026-08-28 저장소 밖 Python 3.14 venv에 LightGBM 4.7.0·XGBoost 3.4.1 기준 worker를 구현했다. 동일한 시간순 train·validation·test와 synthetic PIT 데이터로 text·JSON 아티팩트의 실제 학습·CLI 왕복을 검증했으며 자동 배치·외부 주문 권한은 없다. 수년치 공식 PIT 데이터와 worker 프로세스 자원 강제는 미완료다."
ML_OOS_RECONCILIATION_NOTE = "- 2026-08-28 worker의 test 표본별 하락·횡보·상승 확률을 백만분율로 고정하고 Rust가 log loss·Brier·ECE·balanced accuracy와 fold 수를 독립 재계산한다. Python·Rust 공유 fixture를 통과하며 불일치 결과는 candidate_review 등록 전에 거부한다."
ML_RUNNER_PLAN_NOTE = "- 다음 ML 묶음은 ml-worker-runner-v1이다. 저장된 prepared 작업과 고정 worker 경로만 허용하고 비밀 환경변수 비상속, timeout·출력·결과 크기 제한, Windows Job Object 메모리 상한·자식 종료와 제한된 실패 코드 저장을 함께 구현한다."
ML_RUNNER_DONE_NOTE = "- 2026-08-28 ml-worker-runner-v1을 구현했다. 저장된 prepared 작업과 고정 worker resource만 실행하고 비밀 환경변수를 비상속하며 timeout·출력·결과 크기·비정상 종료를 실패로 닫는다. Windows Job Object로 프로세스·자식 작업 메모리 상한과 종료를 강제하고 실제 아티팩트 해시를 Rust에서 재검증한다."
PIT_DATASET_PLAN_NOTE = "- 다음 ML 묶음은 pit-dataset-builder-v1이다. 자산·주기별 라벨, 비중첩 수집 창, 기업행사·만기·롤오버·펀딩 경계, availableAt 기준 as-of 피처 조인과 재현 가능한 감사·매니페스트 생성을 구현한다. 실제 수년치 공식 데이터 다운로드와 공급자 자격정보는 이 기반과 분리한다."
PIT_DATASET_DONE_NOTE = "- 2026-08-28 pit-dataset-builder-v1을 구현했다. 주식 adjusted close·현물 close·증권선물 settlement·코인 무기한선물 mark 라벨을 고정하고 비중첩 수집 창, 기업행사·만기·롤·펀딩 경계, availableAt as-of 최신 리비전 조인, gap·누수·중복·결측 감사를 기존 Forecast 감사와 ML 불변 매니페스트에 연결했다. 실제 공식 수년치 데이터 다운로드는 아직 미완료다."
PIT_PROVIDER_NOTE = "- 2026-08-28 공식 공개 PIT 페이지 어댑터 1차를 구현했다. Upbit KRW 현물은 배타적 `to`·최대 200봉·역방향 커서를, Binance 현물·USDⓈ-M·COIN-M은 UTC start/end·최대 1,000봉·정방향 커서를 보존한다. 완료 봉만 1e-8 고정소수점과 SHA-256 원천 리비전으로 정규화해 SQLite에 불변 저장하고, 재수집 중복 대사·범위 병합·내부 gap 수를 검사한다. 거래가 없는 Upbit gap은 보간하지 않는다. 토스 주식 수년 이력과 실제 장기 다운로드는 아직 미완료다."
PIT_COLLECTION_JOB_PLAN_NOTE = "- 다음 PIT 묶음은 장기 수집 작업 오케스트레이터다. 한 번에 제한된 페이지만 읽고 SQLite 체크포인트로 재개하며, 원자적 실행권·중복 재생·지수 backoff·취소·실패 상태를 구현한다. 외부 주문과 자격정보는 사용하지 않는다."
PIT_COLLECTION_JOB_DONE_NOTE = "- 2026-08-28 PIT 장기 수집 작업·로컬 반복 스케줄러·저장 범위 데이터셋 조립을 완료했다. 앱 프로세스는 due 작업만 제한적으로 실행하고 재시작 때 SQLite 체크포인트에서 복구한다. 완료 범위는 현재·과거 가격 기반 피처로 기존 감사·불변 매니페스트에 연결하며 외부 주문과 자격정보는 사용하지 않는다."
PIT_LOCAL_SCHEDULER_PLAN_NOTE = "- PIT 장기 수집 작업은 앱 내부 로컬 스케줄러가 due 작업만 제한적으로 실행하고, 종료 후 SQLite 체크포인트에서 재개한다. 별도 Cloud·계정·실주문 권한은 사용하지 않는다."
PIT_STORED_DATASET_PLAN_NOTE = "- 완료된 공식 가격 범위는 원천 리비전을 유지한 결정론적 가격 피처로 변환해 pit-dataset-builder-v1 preview·commit에 연결한다. 부분 조회·gap·자산군 불일치는 fail-closed로 처리한다."
PIT_SHARD_SET_DONE_NOTE = "- 2026-08-28 장기 분봉 데이터셋 shard-set-v1을 구현했다. 기존 감사·불변 매니페스트 2~64개를 동일 자산·split·피처 스키마, split별 엄격한 시간 순서와 표본 ID 비중복 조건으로 묶고 결합 SHA-256과 자식 payload 해시를 조회 때 다시 검증한다. 현재 단일 worker 안전 상한은 유지하며 shard-aware 학습기는 별도 미완료로 fail-closed 처리한다."
PIT_SHARD_WORKER_DONE_NOTE = "- 2026-08-30 XGBoost shard-aware 외부 메모리 worker를 구현했다. Rust runner가 검증된 shard를 고정 실행 폴더에 순차 staging하고 Python은 경로·크기·결합/자식/스키마 해시·split 순서·중복을 재검사한 뒤 DataIter와 ExtMemQuantileDMatrix(hist)로 학습한다. OOS 확률은 기존 Rust 대사를 거쳐 candidate_review로만 등록하며 LightGBM shard 입력은 안전한 streaming 경로가 검증될 때까지 거부한다."
REAL_ML_VALIDATION_NOTE = "- 2026-08-30 Binance 공식 공개 REST의 BTC·ETH 현물과 USDⓈ-M 무기한선물 180일 1시간봉으로 XGBoost 기준 모델을 4개 expanding walk-forward OOS 구간에서 검증했다. 네 조합 모두 학습 클래스 사전확률 기준선보다 log loss는 낮았지만 balanced accuracy가 33.76~37.70%에 그쳐 모델 승격·전략 성과·주문 근거로 인정하지 않는다. 수년치·다주기·비용·레짐·주식 검증은 미완료다."
ML_COST_VALIDATION_NOTE = "- 2026-08-30 OOS 확률을 다음 봉 시가 진입·4시간 뒤 종가 청산의 비중첩 거래로 변환했다. 현물 상승 long과 USDⓈ-M 상승 long·하락 short에 명시적 taker 수수료·슬리피지 1배·1.5배·2배 및 공식 funding 이력을 적용했으며, 1배부터 네 조합 모두 순손실이라 현재 기준 모델을 전략 후보로 기각했다. 정확한 계정 수수료·호가·시장 충격과 수년치·다주기 검증은 미완료다."
ML_MULTITIME_REGIME_NOTE = "- 2026-08-30 Binance BTC·ETH 현물·USDⓈ-M의 730일 1h·4h·1d를 48개 expanding walk-forward 모델로 검증했다. 각 fold의 과거 학습 표본만으로 추세 절대값 중앙값·변동성 75분위수를 산출해 OOS 거래를 6개 관측 레짐으로 분리했고 210개 공식 요청과 12개 조합을 완료했다. 기본 비용에서 전 조합 순손실이라 모델은 계속 기각하며 주식·실계정 비용·호가 체결·3~5년·분봉 검증은 미완료다."
ML_ROLLING_CALIBRATION_NOTE = "- 2026-08-30 현재 fold 이전 OOS 확률만 쓰는 rolling-oos-temperature-v1 진단을 구현하고 Binance 180일 1h 네 조합에서 재검증했다. cold-start를 제외한 조합별 1,289개 동일 표본에서 네 조합 모두 log loss가 악화되어 보정은 채택하지 않았고 원시 모델·전략 신호·주문 잠금은 변경하지 않았다."
ML_SAME_OOS_COMPARISON_NOTE = "- 2026-08-30 동일한 180일 1h·4개 walk-forward fold·조합별 1,719개 OOS ID에서 클래스 사전확률, 학습구간 모멘텀 상태, LightGBM, XGBoost를 비교했다. 현물은 XGBoost, USDⓈ-M은 LightGBM log loss가 근소하게 낮았지만 balanced accuracy가 약 33~37%라 자동 winner·모델 승격·주문 권한을 부여하지 않았다."


def load_api(root: Path) -> ModuleType:
    source = root / "scripts" / "projectstudio_api.py"
    spec = importlib.util.spec_from_file_location("projectstudio_api", source)
    if spec is None or spec.loader is None:
        raise RuntimeError("ProjectStudio 로컬 기획 API를 불러오지 못했습니다.")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def acceptance(feature_id: str, items: list[tuple[str, bool]]) -> list[dict[str, Any]]:
    return [
        {
            "id": f"{feature_id}-ac-{index + 1}",
            "description": description,
            "isMet": is_met,
            "sortOrder": index,
        }
        for index, (description, is_met) in enumerate(items)
    ]


def roadmap_features() -> list[dict[str, Any]]:
    nodes = [
        ("feat-auto-roadmap", "req-broker", "자동매매·모델 고정 개발 로드맵", "틱·분봉 데이터부터 전략·실행·모델·외부 검증까지 고정 순서로 추적한다.", "in_progress", "critical", "소프트웨어 아키텍트 · 매매운영 총괄", 2210, [
            ("판단 주기와 주문 실행 주기를 분리하고 실주문 잠금을 유지한다.", True),
            ("8개 개발 단계와 완료·차단 기준을 저장소 문서와 동일하게 유지한다.", True),
            ("모든 하위 기능이 검증된 뒤에만 로드맵을 완료로 올린다.", False),
        ]),
        ("feat-auto-cadence-contract", "feat-auto-roadmap", "틱·분봉 전략 주기 계약", "전략별 판단 주기와 체결 관리 주기, 지원 자산·공급자와 백테스트 일치 여부를 검증한다.", "done", "critical", "전략 플랫폼 담당 · 시장데이터 엔지니어", 2211, [
            ("tick·1/3/5/15/30분·1/4시간·일봉 판단 주기 계약을 구현한다.", True),
            ("연구원 UI에서 1분봉·일봉 재검증을 선택하고 새 불변 실험으로 저장한다.", True),
            ("백테스트와 운용 interval 불일치를 fail-closed로 차단한다.", True),
            ("현재 완료 일봉 감시의 60초 검사 간격을 명시적으로 보존한다.", True),
        ]),
        ("feat-auto-realtime-aggregation", "feat-auto-roadmap", "실시간 Tick·완료 봉 집계", "공식 WebSocket 입력을 공통 Tick과 완료 봉으로 정규화하고 gap·stale·재연결을 검사한다.", "in_progress", "critical", "시장데이터 엔지니어", 2212, [
            ("Upbit·Binance 공개 스트림 감독기와 stale·재연결 경계를 구현한다.", True),
            ("공통 Tick에서 완료 1분봉과 3·5·15·30·60·240분 봉을 결정론적으로 집계한다.", True),
            ("중복·역순·원천 혼합·overflow를 거부하고 빈 분을 gap으로 보존한다.", True),
            ("공식 WebSocket 수신을 공통 Tick 집계 코어와 연결하고 재시작 상태를 보존한다.", True),
            ("Upbit·Binance 공식 REST로 기록된 gap만 제한적으로 복구하고 무거래 분은 보간하지 않는다.", True),
            ("토스 인증 WebSocket을 Rust 전송으로 연결하고 토큰 비노출·시장 topic 전용·PING·재연결 계약을 구현한다.", True),
            ("토스 인증 WebSocket과 자산별 공식 공급자 왕복을 검증한다.", False),
        ]),
        ("feat-auto-strategy-plugins", "feat-auto-roadmap", "버전형 전략 플러그인", "이동평균 교차 외 전략을 자산·주기·데이터 요구사항과 함께 버전형 계약으로 실행한다.", "done", "high", "퀀트 논문 연구원 · 전략 플랫폼 담당", 2213, [
            ("현재 이동평균 교차를 결정론적 기준 전략으로 유지한다.", True),
            ("추세·돌파·평균회귀·변동성 전략 플러그인 계약을 구현한다.", True),
            ("지원하지 않는 자산·주기·데이터 조합을 거부한다.", True),
        ]),
        ("feat-auto-rust-scheduler", "feat-auto-roadmap", "Rust 상시 섀도우 스케줄러", "UI 타이머와 분리된 Rust worker가 SQLite의 활성 감시를 재시작 뒤 자동 재개한다.", "done", "critical", "매매운영 담당 · 백엔드 담당", 2214, [
            ("Tauri setup에서 중복 없이 백그라운드 감시 worker를 시작한다.", True),
            ("활성 watch를 SQLite에서 읽어 앱 재시작 뒤 자동 재개한다.", True),
            ("동시 tick을 원자적으로 차단하고 UI는 상태만 조회한다.", True),
            ("실전 주문 전송 경로를 추가하지 않고 후보별 사용자 승인을 유지한다.", True),
        ]),
        ("feat-auto-execution-algorithms", "feat-auto-roadmap", "내부 주문 실행 알고리즘", "지정가·재호가·부분체결·취소와 자산별 체결 제약을 내부 모의원장에서 검증한다.", "done", "critical", "매매운영 담당 · 리스크관리 총괄", 2215, [
            ("시장가 내부 체결과 지정가 대기·취소를 구현한다.", True),
            ("재호가·부분체결·분할 주문·최대 슬리피지 계약을 구현한다.", True),
            ("선물·무기한선물의 증거금·reduce-only·청산 경계를 연결한다.", True),
        ]),
        ("feat-auto-promotion-deployment", "feat-auto-roadmap", "전략 승격·자동 배치·롤백", "검증된 전략을 사용자 승인 뒤 섀도우·내부 모의운용에 배치하고 중지·롤백한다.", "done", "critical", "독립 모델검증 담당 · 매매운영 총괄", 2216, [
            ("백테스트 결과를 승인 대기 내부 모의주문 후보로 연결한다.", True),
            ("OOS·Walk-forward·비용 스트레스와 전략 버전을 배치에 고정한다.", True),
            ("Canary·성과 악화 정지·이전 버전 롤백을 구현한다.", True),
        ]),
        ("feat-auto-ml-model-development", "feat-auto-roadmap", "실제 ML 예측 모델 개발", "PIT 데이터셋·기준 모델·외부 worker·모델 레지스트리와 보정된 확률 추론을 구현한다.", "in_progress", "high", "모델·전략 MLOps 담당 · 독립 모델검증 담당", 2217, [
            ("확률 저장·데이터 품질 감사·보정·승격 게이트 기반을 구현한다.", True),
            ("수년치 PIT 데이터셋과 LightGBM/XGBoost 기준 모델을 구현한다.", False),
            ("Chronos·TimesFM 비교 worker와 모델 레지스트리·재학습을 구현한다.", False),
        ]),
        ("feat-auto-ml-worker-foundation", "feat-auto-ml-model-development", "PIT 데이터 매니페스트·ML worker 계약", "감사된 데이터와 피처 스키마를 고정하고 격리 worker 요청·결과·모델 검토 후보를 추적한다.", "done", "high", "모델·전략 MLOps 담당 · 데이터 품질 담당", 22171, [
            ("시간순 split과 타깃 관측 경계를 검사해 미래 정보 누수를 차단한다.", True),
            ("데이터·피처·코드·seed·파라미터를 SHA-256 계보로 고정한다.", True),
            ("worker 실패와 허용 포맷의 성공 결과를 분리하고 모델을 검토 후보로만 등록한다.", True),
            ("실주문·자동 배치·임의 경로·쉘 명령과 pickle을 허용하지 않는다.", True),
        ]),
        ("feat-auto-ml-baseline-worker", "feat-auto-ml-model-development", "LightGBM·XGBoost 기준 worker", "격리 Python 환경에서 두 기준 모델을 동일 PIT 계약과 시간순 OOS split으로 실제 학습한다.", "done", "high", "모델·전략 MLOps 담당 · 독립 모델검증 담당", 22172, [
            ("LightGBM·XGBoost 공식 wheel 버전을 lockfile에 고정하고 저장소 밖 venv에서 실행한다.", True),
            ("데이터·피처 스키마·작업 해시와 미래 정보 누수를 worker에서 재검사한다.", True),
            ("test 구간에서만 log loss·Brier·ECE·balanced accuracy를 계산한다.", True),
            ("text·JSON 아티팩트와 CLI 완료 JSON을 실제 두 모델 학습으로 검증한다.", True),
        ]),
        ("feat-auto-ml-oos-reconciliation", "feat-auto-ml-model-development", "OOS 원시 확률 Rust 재계산", "worker의 test 원시 확률에서 핵심 OOS 지표를 Rust가 다시 계산해 보고값을 신뢰하지 않고 대사한다.", "done", "high", "독립 모델검증 담당 · 데이터 품질 담당", 22173, [
            ("하락·횡보·상승 확률을 백만분율 정수로 고정하고 합계 1,000,000을 강제한다.", True),
            ("표본 ID·fold·정답 클래스·확률 범위를 Rust에서 검증한다.", True),
            ("log loss·Brier·ECE·balanced accuracy를 Rust에서 독립 재계산한다.", True),
            ("Python·Rust 공유 fixture와 지표 변조 거부 회귀검사를 통과한다.", True),
        ]),
        ("feat-auto-ml-worker-runner", "feat-auto-ml-model-development", "ML worker 실행 격리·자원 제한", "저장된 prepared 작업만 고정 worker로 실행하고 timeout·출력·메모리 상한과 실패 복구를 강제한다.", "done", "high", "모델·전략 MLOps 담당 · 보안 담당", 22174, [
            ("임의 실행 경로·스크립트·인자를 받지 않고 앱이 고정한 worker만 실행한다.", True),
            ("금융·Cloud·GitHub·Telegram 비밀 환경변수를 worker에 상속하지 않는다.", True),
            ("timeout·stdout·stderr·결과 JSON 크기와 비정상 종료를 fail-closed로 처리한다.", True),
            ("Windows Job Object로 메모리 상한과 앱 종료 시 자식 종료를 강제한다.", True),
            ("같은 prepared 작업의 동시 실행을 거부하고 재시작 뒤 새 attempt로 복구한다.", True),
            ("정상·timeout·출력 초과·잘못된 JSON·비정상 종료 회귀검사를 통과한다.", True),
        ]),
        ("feat-auto-ml-pit-dataset-builder", "feat-auto-ml-model-development", "PIT 데이터·라벨 빌더", "공급자와 분리된 자산·주기별 라벨·수집창·시장 경계·as-of 피처 조인·감사·불변 매니페스트 계약을 구현한다.", "done", "high", "데이터 품질 담당 · 모델·전략 MLOps 담당", 22175, [
            ("주식·코인 현물·증권 선물·코인 무기한선물의 가격 기준과 라벨 horizon을 명시한다.", True),
            ("수년 범위를 비중첩·재현 가능한 수집 창으로 나누고 페이지 경계를 검증한다.", True),
            ("기업행사·만기·롤오버·펀딩 경계를 표본 생성에서 fail-closed로 처리한다.", True),
            ("availableAt 이하 최신 리비전만 쓰는 as-of 피처 조인과 누수·중복·결측 감사를 구현한다.", True),
            ("기존 Forecast 감사와 ML 불변 매니페스트 저장 경로에 연결한다.", True),
            ("실제 수년치 공식 데이터 다운로드와 공급자 자격정보 연결은 별도 미완료로 유지한다.", True),
        ]),
        ("feat-auto-ml-pit-provider-pages", "feat-auto-ml-model-development", "공식 공급자 PIT 페이지 수집", "공식 읽기 전용 API의 서로 다른 페이지 방향·한도·완료 시각을 보존해 공급자 독립 가격 관측으로 정규화한다.", "in_progress", "high", "시장데이터 엔지니어 · 데이터 품질 담당", 22176, [
            ("Upbit KRW 현물의 배타적 to·최대 200봉·역방향 페이지 계약을 구현한다.", True),
            ("Binance 현물·USDⓈ-M·COIN-M의 UTC start/end·정방향 페이지 계약을 구현한다.", True),
            ("완료 봉만 1e-8 고정소수점과 SHA-256 원천 리비전으로 정규화한다.", True),
            ("거래가 없어 생성되지 않은 Upbit 봉을 임의 보간하지 않고 gap으로 남긴다.", True),
            ("페이지를 불변 SQLite 관측으로 저장하고 재수집 중복·원천 리비전 변조를 거부한다.", True),
            ("저장된 페이지를 시간순 범위로 병합하고 내부 gap·조회 절단 여부를 보고한다.", True),
            ("토스 주식 공식 수년 이력 페이지 계약과 실제 장기 다운로드를 검증한다.", False),
        ]),
        ("feat-auto-ml-pit-collection-jobs", "feat-auto-ml-pit-provider-pages", "PIT 장기 수집 작업·재시작 복구", "공식 공개 페이지를 제한된 batch로 수집하고 SQLite 체크포인트에서 중단·재시작·재시도·취소한다.", "done", "high", "시장데이터 엔지니어 · SRE · 데이터 품질 담당", 22177, [
            ("멱등키와 고정 요청 해시로 장기 수집 작업을 생성한다.", True),
            ("한 번에 최대 5페이지만 실행하고 공급자 호출 한도를 넘지 않는다.", True),
            ("원자적 실행권으로 동시 실행을 막고 앱 중단 뒤 체크포인트에서 재개한다.", True),
            ("일시 오류는 최대 4회 지수 backoff 후 재시도하고 영구 오류는 실패로 닫는다.", True),
            ("취소·완료·실패 상태와 페이지·관측 수를 SQLite에서 재생한다.", True),
            ("실주문·출금·개인 계좌 자격정보를 사용하지 않는다.", True),
        ]),
        ("feat-auto-ml-pit-local-scheduler", "feat-auto-ml-pit-collection-jobs", "PIT 로컬 반복 수집 스케줄러", "앱 프로세스의 백그라운드 worker가 due 작업만 실행하고 SQLite lease·체크포인트에서 재시작 복구한다.", "done", "high", "시장데이터 엔지니어 · SRE", 22178, [
            ("앱 시작 시 스케줄러를 한 번만 시작하고 UI 수명과 분리한다.", True),
            ("queued·도래한 retry_wait·stale running 작업만 제한된 수로 선택한다.", True),
            ("수집 명령과 같은 실행권·호출 제한·체크포인트 경로를 재사용한다.", True),
            ("앱 재시작 뒤 완료·취소 작업은 건드리지 않고 미완료 작업만 재개한다.", True),
            ("외부 서버·계정·실주문·출금 권한을 사용하지 않는다.", True),
        ]),
        ("feat-auto-ml-pit-stored-dataset", "feat-auto-ml-pit-dataset-builder", "저장 PIT 범위 데이터셋 자동 조립", "불변 저장된 공식 가격 범위를 시점 정합 파생 피처와 함께 기존 데이터셋 preview·commit 계약으로 조립한다.", "done", "high", "데이터 품질 담당 · 모델·전략 MLOps 담당", 22179, [
            ("공급자·심볼·주기·자산 계약 일치를 검증한다.", True),
            ("현재와 과거 가격만 사용하는 return·이동평균 괴리 피처를 결정론적으로 만든다.", True),
            ("원천 리비전·availableAt·ingestedAt 계보를 파생 피처에 보존한다.", True),
            ("조회 절단·24시간 시장 gap·부족한 warmup은 데이터셋 생성 전에 거부한다.", True),
            ("기존 Forecast 감사·ML 불변 매니페스트 preview·commit 경로를 재사용한다.", True),
            ("모델 자동 승격과 외부 주문 권한을 부여하지 않는다.", True),
        ]),
        ("feat-auto-ml-pit-shard-set", "feat-auto-ml-pit-dataset-builder", "장기 분봉 데이터셋 shard set", "단일 매니페스트 안전 상한을 유지하면서 검증된 불변 매니페스트를 논리 데이터셋으로 묶어 재생·변조 검증한다.", "done", "high", "데이터 품질 담당 · 모델·전략 MLOps 담당", 22180, [
            ("기존 감사와 불변 저장을 통과한 매니페스트 2~64개만 구성원으로 허용한다.", True),
            ("모든 shard의 자산 계약·split·피처 스키마가 동일한지 검사한다.", True),
            ("train·validation·test 각각의 shard 시간이 엄격히 증가하고 표본 ID가 중복되지 않는지 검사한다.", True),
            ("구성 순서와 자식 해시를 결합 SHA-256으로 고정하고 동일 재시도를 멱등 처리한다.", True),
            ("상세·이력 재생 때 결합 해시와 각 자식 payload 해시를 다시 검증한다.", True),
            ("XGBoost worker에는 고정 파일 목록으로만 전달하고 LightGBM shard 입력은 명시적으로 거부한다.", True),
            ("자동 승격·외부 주문·출금 권한을 부여하지 않는다.", True),
        ]),
        ("feat-auto-ml-xgboost-external-memory-worker", "feat-auto-ml-model-development", "XGBoost shard-aware 외부 메모리 worker", "검증된 장기 분봉 shard를 한 번에 합치지 않고 고정 파일 목록과 외부 메모리 학습기로 순차 소비한다.", "done", "high", "모델·전략 MLOps 담당 · 데이터 품질 담당 · 보안 담당", 22181, [
            ("Rust runner가 검증된 shard payload만 작업별 고정 폴더에 create-new 방식으로 staging하고 실행 뒤 제거한다.", True),
            ("Python이 결합·자식·스키마 해시, 파일명·경로 경계, 크기, split 시간 순서와 표본 중복을 재검사한다.", True),
            ("XGBoost DataIter·ExtMemQuantileDMatrix와 hist tree method로 shard를 순차 학습한다.", True),
            ("test OOS 확률을 기존 Rust 지표 재계산과 candidate_review 등록 경로에 연결한다.", True),
            ("LightGBM shard 입력과 자동 승격·외부 주문·출금 권한을 fail-closed로 유지한다.", True),
            ("실제 모델 생성, 변조·경로 이탈·계보 불일치 회귀검사를 통과한다.", True),
        ]),
        ("feat-auto-ml-real-market-validation", "feat-auto-ml-model-development", "공식 실제시장 ML 기준 검증", "공식 공개 실제 가격·펀딩·mark/index 데이터로 시간순 OOS 모델을 실행하고 단순 기준선과 비교한다.", "in_progress", "high", "독립 모델검증 담당 · 데이터 품질 담당", 22182, [
            ("BTC·ETH 현물과 USDⓈ-M 180일 1시간봉을 공식 공개 API에서 수집한다.", True),
            ("4개 expanding walk-forward OOS 구간의 표본 비중복·해시·시점 경계를 검사한다.", True),
            ("XGBoost 확률 지표를 학습 클래스 사전확률 기준선과 비교하고 승격 없이 기록한다.", True),
            ("비중첩 OOS 거래에 1배·1.5배·2배 비용과 공식 funding을 적용해 현재 모델을 기각한다.", True),
            ("730일 1h·4h·1d를 fold별 과거 학습 기준의 6개 관측 레짐으로 재검증한다.", True),
            ("3~5년·분봉·주식 데이터와 실제 계정 비용·호가 체결에서 재검증한다.", False),
        ]),
        ("feat-auto-ml-rolling-oos-calibration", "feat-auto-ml-model-development", "Rolling OOS 확률 보정 진단", "현재 fold 이전 OOS 확률만으로 다중분류 temperature를 맞추고 다음 fold의 원시·보정 확률을 동일 표본에서 비교한다.", "done", "high", "독립 모델검증 담당 · 데이터 품질 담당", 22183, [
            ("첫 OOS fold를 cold-start로 분리하고 현재·미래 fold를 보정 학습에서 거부한다.", True),
            ("단일 temperature가 argmax와 확률 순위를 보존하고 백만분율 합계 1,000,000을 유지한다.", True),
            ("동일 OOS 표본의 log loss·Brier·ECE·balanced accuracy 전후를 기록한다.", True),
            ("180일 1h 네 조합에서 악화 결과를 숨기지 않고 보정 미채택으로 기록한다.", True),
            ("자동 승격·전략 신호 변경·외부 주문 권한을 부여하지 않는다.", True),
        ]),
        ("feat-auto-ml-same-oos-model-comparison", "feat-auto-ml-model-development", "동일 OOS 기준 모델 비교", "같은 fold·피처·horizon·OOS ID에서 단순 기준선과 LightGBM·XGBoost 확률 지표를 비교한다.", "done", "high", "독립 모델검증 담당 · 모델·전략 MLOps 담당", 22184, [
            ("학습구간 return_4 상태별 클래스 빈도에만 Laplace smoothing을 적용한 단순 기준선을 만든다.", True),
            ("기존 shard XGBoost와 단일 manifest LightGBM을 같은 split과 OOS ID에서 실행한다.", True),
            ("log loss·Brier·ECE·balanced accuracy를 동일 표본에서 비교한다.", True),
            ("180일 1h 네 조합의 32개 실제 모델과 OOS 표본 순서 대사를 완료한다.", True),
            ("최저 log loss 표시는 설명용으로만 두고 자동 winner·승격·주문을 금지한다.", True),
        ]),
        ("feat-auto-external-soak", "feat-auto-roadmap", "외부 모의연결·24시간 내구 검증", "공식 공급자와 모의계좌를 연결하고 재시작·단절·stale·중복을 실제 시간으로 검증한다.", "planned", "high", "외부 어댑터 담당 · 운영 담당", 2218, [
            ("Toss·KIS·Binance Testnet 등 승인된 외부 모의 경로를 검증한다.", False),
            ("내부 섀도우 실제 시간 표본 수집·재시작 대사·표본 공백 fail-closed 기반을 구현한다.", True),
            ("24시간 스트림·섀도우·재시작 soak test를 통과한다.", False),
            ("실주문·출금은 계속 잠그고 Cloud relay는 라우팅 해결 뒤 재개한다.", True),
        ]),
    ]
    return [
        {
            "id": feature_id,
            "parentId": parent_id,
            "title": title,
            "description": description,
            "status": status,
            "priority": priority,
            "role": role,
            "sortOrder": sort_order,
            "colorKey": "green" if status == "done" else "amber" if status == "planned" else "cyan",
            "acceptanceCriteria": acceptance(feature_id, criteria),
        }
        for feature_id, parent_id, title, description, status, priority, role, sort_order, criteria in nodes
    ]


def normalize_flow(current: dict[str, Any]) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    nodes = []
    for node in current["userFlow"]["nodes"]:
        metadata = json.loads(node.get("metadata_json") or "{}")
        nodes.append({
            "id": node["id"], "laneId": node["lane_id"], "title": node["title"], "description": node["description"], "kind": node["kind"],
            "positionX": node["position_x"], "positionY": node["position_y"], "colorKey": node.get("color_key") or "violet", "depth": node.get("depth"),
            "parentId": node.get("parent_id"), "linkedFeatureIds": json.loads(node.get("linked_feature_ids") or "[]"), "branchCondition": node.get("branch_condition"),
            "inputArtifacts": metadata.get("inputArtifacts", []), "outputArtifacts": metadata.get("outputArtifacts", []), "methods": metadata.get("methods", []),
            "validation": metadata.get("validation", ""), "failureHandling": metadata.get("failureHandling", ""), "codePaths": metadata.get("codePaths", []),
            "testPaths": metadata.get("testPaths", []), "completionCriteria": metadata.get("completionCriteria", ""), "isCompleted": bool(metadata.get("isCompleted", False)),
        })
    edges = [{"id": edge["id"], "sourceNodeId": edge["source_node_id"], "targetNodeId": edge["target_node_id"]} for edge in current["userFlow"]["edges"]]
    return nodes, edges


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("projectstudio_root", type=Path)
    args = parser.parse_args()
    api = load_api(args.projectstudio_root)
    database = api.default_database_path()
    with api.connect_database(database, writable=True) as connection:
        current = api.get_project(connection, PROJECT_ID)
        desired = roadmap_features()
        replacements = {item["id"]: item for item in desired}
        existing = {item["id"]: item for item in current["features"]}
        keys = ("parentId", "title", "description", "status", "priority", "role", "sortOrder", "colorKey", "acceptanceCriteria")
        changed = any(item["id"] not in existing or any(existing[item["id"]].get(key) != item.get(key) for key in keys) for item in desired)
        merged = [replacements.pop(item["id"], item) for item in current["features"]]
        merged.extend(replacements.values())
        original_markdown = current["project"]["prd_markdown"]
        markdown = original_markdown.replace(OLD_AGGREGATION_NOTE, AGGREGATION_NOTE).replace(OLD_PUBLIC_STREAM_PENDING_NOTE, PUBLIC_STREAM_NOTE).replace(OLD_ML_WORKER_NOTE, ML_WORKER_NOTE).replace(OLD_ML_BASELINE_NOTE, ML_BASELINE_NOTE).replace(ML_RUNNER_PLAN_NOTE, ML_RUNNER_DONE_NOTE).replace(PIT_DATASET_PLAN_NOTE, PIT_DATASET_DONE_NOTE).replace(PIT_COLLECTION_JOB_PLAN_NOTE, PIT_COLLECTION_JOB_DONE_NOTE)
        if PIT_SHARD_SET_DONE_NOTE in markdown and PIT_SHARD_WORKER_DONE_NOTE not in markdown:
            markdown = markdown.replace(PIT_SHARD_SET_DONE_NOTE, f"{PIT_SHARD_SET_DONE_NOTE}\n{PIT_SHARD_WORKER_DONE_NOTE}")
        if TOSS_WEBSOCKET_NOTE in markdown and SHADOW_SOAK_HARNESS_NOTE not in markdown:
            markdown = markdown.replace(TOSS_WEBSOCKET_NOTE, f"{TOSS_WEBSOCKET_NOTE}\n{SHADOW_SOAK_HARNESS_NOTE}")
        section_missing = SECTION_MARKER not in markdown
        if section_missing:
            markdown += f"""

## {SECTION_MARKER}
- 고정 순서: 전략 주기 계약 → Tick·완료 봉 집계 → 전략 플러그인 → Rust 상시 스케줄러 → 내부 주문 실행 → 승격·자동 배치·롤백 → ML 모델 → 외부 연결·24시간 검증.
- LLM·ML은 분석과 제안만 수행하며 빠른 판단·위험 제한·체결은 결정론적 Rust 알고리즘이 담당한다.
- 판단 주기와 체결 관리 주기를 분리하고, 백테스트 interval과 운용 interval이 다르면 실행을 차단한다.
- 사용자 승인 없이 실주문·출금을 열지 않으며 현재 배치는 내부 섀도우와 모의원장으로 제한한다.
- 2026-08-27 이동평균 섀도우 감시를 React 타이머에서 Rust 백그라운드 worker로 이전하고 저장 백테스트의 1m·1d interval을 fresh bar 조회에 고정했다. UI 연구 생성은 아직 일봉이며 틱 지원 완료를 뜻하지 않는다.
- 2026-08-28 연구원 패널에 1m·1d 재검증 선택을 추가했다. 기존 결과는 수정하지 않고 interval이 포함된 새 experiment·dataset ID로 저장하며 섀도우 감시는 같은 interval을 사용한다. 1분봉 200개는 짧은 탐색 구간으로 경고하고 성과 승격 근거로 사용하지 않는다.
- 2026-08-28 공통 Tick·완료 봉 Rust 집계 코어를 추가했다. 1분봉과 3·5·15·30·60·240분 봉, partial 분리, gap 보존, 중복·역순·overflow 거부를 검증했다.
{PUBLIC_STREAM_NOTE}
{TOSS_WEBSOCKET_NOTE}
{SHADOW_SOAK_HARNESS_NOTE}
- 2026-08-28 이동평균 교차·가격 채널 돌파·평균 이격 회귀·ATR 변동성 확장을 v1 순수 Rust 플러그인으로 구현했다. 백테스트와 섀도우 최신 신호가 같은 디스패처를 사용하며 혼합 플러그인·미지원 주기·필드 누락을 사전 거부한다.
{CADENCE_CONTRACT_NOTE}
- 2026-08-28 internal-execution-v1을 구현했다. 최소 수량 단위 기반 분할, 최초 기준가 대비 최대 슬리피지, 재호가 횟수, 명시적 부분체결, 취소·만료와 멱등 사건을 SQLite에 보존한다. 증권 선물·코인 무기한선물은 최대 2배 격리증거금·청산 완충·reduce-only 포지션 감축을 통과해야 하며 외부 주문 전송은 없다.
- 2026-08-28 strategy-deployment-v1을 구현했다. 저장된 OOS·Walk-forward 전 항목과 1.5배·2배 비용 스트레스를 다시 검증하고 experiment·dataset·전략 스키마·플러그인 버전을 SHA-256 근거로 고정한다. 명시적 승인 뒤 SHADOW Canary, 관측 기반 자동 중지, 별도 승인형 내부 모의운용과 직전 버전 롤백을 SQLite 사건으로 보존하며 외부 주문 전송은 없다.
- 2026-08-28 investa-ml-worker-v1 기반을 구현했다. PIT 품질 감사를 통과한 데이터 payload·피처 스키마와 학습 code·seed·horizon·파라미터를 SHA-256으로 고정하고 split 타깃 누수·변조를 거부한다. 성공 결과는 허용 아티팩트·OOS 지표 기반 candidate_review로만 등록한다.
- 2026-08-28 저장소 밖 Python 3.14 venv에 LightGBM 4.7.0·XGBoost 3.4.1 기준 worker를 구현했다. 동일한 시간순 train·validation·test와 synthetic PIT 데이터로 text·JSON 아티팩트의 실제 학습·CLI 왕복을 검증했으며 자동 배치·외부 주문 권한은 없다. 수년치 공식 PIT 데이터와 worker 프로세스 자원 강제는 미완료다.
- 2026-08-28 worker의 test 표본별 하락·횡보·상승 확률을 백만분율로 고정하고 Rust가 log loss·Brier·ECE·balanced accuracy와 fold 수를 독립 재계산한다. Python·Rust 공유 fixture를 통과하며 불일치 결과는 candidate_review 등록 전에 거부한다.
- 2026-08-28 ml-worker-runner-v1을 구현했다. 저장된 prepared 작업과 고정 worker resource만 실행하고 비밀 환경변수를 비상속하며 timeout·출력·결과 크기·비정상 종료를 실패로 닫는다. Windows Job Object로 프로세스·자식 작업 메모리 상한과 종료를 강제하고 실제 아티팩트 해시를 Rust에서 재검증한다.
- 2026-08-28 pit-dataset-builder-v1을 구현했다. 주식 adjusted close·현물 close·증권선물 settlement·코인 무기한선물 mark 라벨을 고정하고 비중첩 수집 창, 기업행사·만기·롤·펀딩 경계, availableAt as-of 최신 리비전 조인, gap·누수·중복·결측 감사를 기존 Forecast 감사와 ML 불변 매니페스트에 연결했다. 실제 공식 수년치 데이터 다운로드는 아직 미완료다.
- 2026-08-28 공식 공개 PIT 페이지 어댑터 1차를 구현했다. Upbit KRW 현물은 배타적 `to`·최대 200봉·역방향 커서를, Binance 현물·USDⓈ-M·COIN-M은 UTC start/end·최대 1,000봉·정방향 커서를 보존한다. 완료 봉만 1e-8 고정소수점과 SHA-256 원천 리비전으로 정규화해 SQLite에 불변 저장하고, 재수집 중복 대사·범위 병합·내부 gap 수를 검사한다. 거래가 없는 Upbit gap은 보간하지 않는다. 토스 주식 수년 이력과 실제 장기 다운로드는 아직 미완료다.
- 2026-08-28 PIT 장기 수집 작업·로컬 반복 스케줄러·저장 범위 데이터셋 조립을 완료했다. 앱 프로세스는 due 작업만 제한적으로 실행하고 재시작 때 SQLite 체크포인트에서 복구한다. 완료 범위는 현재·과거 가격 기반 피처로 기존 감사·불변 매니페스트에 연결하며 외부 주문과 자격정보는 사용하지 않는다.
- 2026-08-28 장기 분봉 데이터셋 shard-set-v1을 구현했다. 기존 감사·불변 매니페스트 2~64개를 동일 자산·split·피처 스키마, split별 엄격한 시간 순서와 표본 ID 비중복 조건으로 묶고 결합 SHA-256과 자식 payload 해시를 조회 때 다시 검증한다. 현재 단일 worker 안전 상한은 유지하며 shard-aware 학습기는 별도 미완료로 fail-closed 처리한다.
{PIT_SHARD_WORKER_DONE_NOTE}
{REAL_ML_VALIDATION_NOTE}
{ML_COST_VALIDATION_NOTE}
{ML_MULTITIME_REGIME_NOTE}
{ML_ROLLING_CALIBRATION_NOTE}
{ML_SAME_OOS_COMPARISON_NOTE}
"""
        else:
            missing_notes = [
                note for note in (AGGREGATION_NOTE, PUBLIC_STREAM_NOTE, REST_GAP_BACKFILL_NOTE, TOSS_WEBSOCKET_NOTE, STRATEGY_PLUGIN_NOTE, CADENCE_CONTRACT_NOTE, EXECUTION_ALGORITHM_NOTE, STRATEGY_DEPLOYMENT_NOTE, ML_WORKER_NOTE, ML_BASELINE_NOTE, ML_OOS_RECONCILIATION_NOTE, ML_RUNNER_DONE_NOTE, PIT_DATASET_DONE_NOTE, PIT_PROVIDER_NOTE, PIT_COLLECTION_JOB_DONE_NOTE, PIT_LOCAL_SCHEDULER_PLAN_NOTE, PIT_STORED_DATASET_PLAN_NOTE, PIT_SHARD_SET_DONE_NOTE, PIT_SHARD_WORKER_DONE_NOTE, REAL_ML_VALIDATION_NOTE, ML_COST_VALIDATION_NOTE, ML_MULTITIME_REGIME_NOTE, ML_ROLLING_CALIBRATION_NOTE, ML_SAME_OOS_COMPARISON_NOTE)
                if note not in markdown
            ]
            if missing_notes:
                marker_index = markdown.index(SECTION_MARKER)
                section_start = markdown.rfind("\n", 0, marker_index) + 1
                section_end = markdown.find("\n## ", marker_index + len(SECTION_MARKER))
                if section_end < 0:
                    section_end = len(markdown)
                insertion = "\n" + "\n".join(missing_notes)
                markdown = markdown[:section_end].rstrip() + insertion + "\n" + markdown[section_end:].lstrip("\n")
        changed = changed or markdown != original_markdown
        if not changed:
            print(json.dumps({"projectId": PROJECT_ID, "committed": False, "message": "동일한 자동매매 로드맵이 이미 반영되어 있습니다."}, ensure_ascii=False, indent=2))
            return
        nodes, edges = normalize_flow(current)
        bundle = {
            "schemaVersion": 1,
            "projectId": PROJECT_ID,
            "expectedPrdRevisionNumber": current["project"]["revision_number"],
            "prd": {"title": current["project"]["prd_title"], "markdown": markdown},
            "features": merged,
            "userFlow": {"nodes": nodes, "edges": edges},
        }
        api.validate_bundle(bundle)
        print(json.dumps(api.apply_bundle(connection, database, bundle, commit=True), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
