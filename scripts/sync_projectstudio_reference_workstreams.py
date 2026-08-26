"""유명 레퍼런스 검토와 신규 강건성 기능명세를 ProjectStudio에 멱등 반영한다."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path
from types import ModuleType
from typing import Any


PROJECT_ID = "36e87491-74a8-48ca-a7b8-30fa6ccea131"
SECTION_MARKER = "유명 레퍼런스 적용과 4개 개발 작업군"
RUNTIME_SECTION_MARKER = "위험정책 운영 연결과 강건성 가시화"


def load_api(projectstudio_root: Path) -> ModuleType:
    source = projectstudio_root / "scripts" / "projectstudio_api.py"
    spec = importlib.util.spec_from_file_location("projectstudio_api", source)
    if spec is None or spec.loader is None:
        raise RuntimeError("ProjectStudio 로컬 기획 API를 불러오지 못했습니다.")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def acceptance(feature_id: str, descriptions: list[str], met: list[bool]) -> list[dict[str, Any]]:
    return [
        {
            "id": f"{feature_id}-ac-{index + 1}",
            "description": description,
            "isMet": met[index],
            "sortOrder": index,
        }
        for index, description in enumerate(descriptions)
    ]


def feature_nodes() -> list[dict[str, Any]]:
    return [
        {
            "id": "feat-reference-workstream-classification",
            "parentId": "req-project-records",
            "title": "유명 레퍼런스 검토와 4개 개발 작업군",
            "description": "추가 기능을 즉시 개발, Codex 연결, 계좌·외부 자격정보 필요, 모델 연구로 분류하고 저장소·라이선스·데이터 가정·적용 여부를 기록한다.",
            "status": "done",
            "priority": "high",
            "role": "소프트웨어 아키텍트 · 퀀트 논문 연구원",
            "sortOrder": 2110,
            "colorKey": "green",
            "acceptanceCriteria": acceptance(
                "feat-reference-workstream-classification",
                [
                    "LEAN·Qlib·Freqtrade·NautilusTrader와 Chronos·TimesFM의 공식 저장소, 라이선스와 적용 경계를 기록한다.",
                    "새 의존성·외부 코드를 무단 도입하지 않고 즉시 개발·Codex·계좌·모델 작업을 분리한다.",
                    "검증된 구현 범위만 완료로 표시하고 계정·모델 미연결 상태를 유지한다.",
                ],
                [True, True, True],
            ),
        },
        {
            "id": "feat-backtest-bootstrap-robustness",
            "parentId": "req-research",
            "title": "백테스트 경험적 부트스트랩 강건성",
            "description": "완료 거래 손익을 고정 seed로 2,000회 재표집해 수익률 분위, 손실 확률, 초기자금 50% 이하 도달 확률과 최악 경로 낙폭을 계산한다.",
            "status": "done",
            "priority": "high",
            "role": "퀀트 논문 연구원",
            "sortOrder": 2111,
            "colorKey": "green",
            "acceptanceCriteria": acceptance(
                "feat-backtest-bootstrap-robustness",
                [
                    "같은 실험·데이터·거래는 같은 seed와 같은 결과를 만든다.",
                    "완료 거래 5건 미만이면 확률과 분위수를 만들지 않는다.",
                    "결과가 자동 승격·주문 권한을 부여하지 않고 IID 재표집 한계를 표시한다.",
                ],
                [True, True, True],
            ),
        },
        {
            "id": "feat-portfolio-risk-analytics-backend",
            "parentId": "req-performance",
            "title": "포트폴리오 VaR·CVaR·상관·스트레스 분석",
            "description": "동일 통화·동일 관측 시점 수익률로 역사적 VaR·CVaR, 집중도 HHI, 상관과 명시적 충격 손익을 계산한다.",
            "status": "done",
            "priority": "high",
            "role": "성과분석가 · 리스크관리 총괄",
            "sortOrder": 2112,
            "colorKey": "green",
            "acceptanceCriteria": acceptance(
                "feat-portfolio-risk-analytics-backend",
                [
                    "관측 시각 정렬·가용 시각·비중 10,000bp 계약을 검증한다.",
                    "시점 정합 환율 없이 혼합 통화를 합산하지 않는다.",
                    "30개 미만 표본의 95% 위험과 100개 미만 표본의 99% VaR를 꾸며내지 않는다.",
                    "동일 스냅샷 ID의 입력과 결과를 불변 저장하고 이력을 재생한다.",
                ],
                [True, True, True, True],
            ),
        },
        {
            "id": "feat-strategy-protection-evaluator",
            "parentId": "req-risk",
            "title": "전략 쿨다운·손실 보호 평가 계약",
            "description": "쿨다운, 반복 손절, 연속 손실, 최대 낙폭과 종목별 저수익을 전역·종목 잠금으로 결정론적으로 평가한다.",
            "status": "done",
            "priority": "critical",
            "role": "리스크관리 총괄",
            "sortOrder": 2113,
            "colorKey": "green",
            "acceptanceCriteria": acceptance(
                "feat-strategy-protection-evaluator",
                [
                    "미래·역순·식별자 오류 거래 사건을 거절한다.",
                    "보호 사유·관측값·기준·잠금 범위를 구조화한다.",
                    "평가 결과의 실전 주문 허용은 항상 false다.",
                ],
                [True, True, True],
            ),
        },
        {
            "id": "feat-strategy-protection-runtime-integration",
            "parentId": "req-risk",
            "title": "전략 보호장치 섀도우·모의후보 연결",
            "description": "승인된 보호 정책을 섀도우 감시와 내부 모의주문 후보 직전에 적용하고 잠금·해제·거절 사건을 불변 저장한다.",
            "status": "done",
            "priority": "critical",
            "role": "리스크관리 총괄 · 매매운영 담당",
            "sortOrder": 2114,
            "colorKey": "green",
            "acceptanceCriteria": acceptance(
                "feat-strategy-protection-runtime-integration",
                [
                    "활성 사용자 승인 정책만 주문 후보 직전 평가한다.",
                    "전역·종목 잠금 중 신규 포지션 후보를 차단하고 청산·취소는 막지 않는다.",
                    "재시작 후 잠금 만료와 사유가 원장 재생으로 복원된다.",
                    "성과 화면에서 보호정책 사용 여부와 기간·횟수·낙폭 기준을 추천안에 포함할 수 있다.",
                ],
                [True, True, True, True],
            ),
        },
        {
            "id": "feat-forecast-foundation-model-adapters",
            "parentId": "feat-research-probabilistic-forecast",
            "title": "Chronos·TimesFM 확률 예측 어댑터",
            "description": "Apache-2.0 공식 모델을 교체 가능한 외부 Python worker로 연결하고 자산군·horizon별 분위 예측을 기존 PIT·보정·OOS 계약으로 검증한다.",
            "status": "planned",
            "priority": "high",
            "role": "모델·전략 MLOps 담당 · 독립 모델검증 담당",
            "sortOrder": 2115,
            "colorKey": "amber",
            "acceptanceCriteria": acceptance(
                "feat-forecast-foundation-model-adapters",
                [
                    "모델·가중치·Python 환경 설치를 사용자 승인 후 격리한다.",
                    "model/version/dataset/asset/asOf/horizon/quantile/seed를 불변 기록한다.",
                    "자산군별 OOS 보정과 순진 기준선을 이기지 못하면 승격하지 않는다.",
                ],
                [False, False, False],
            ),
        },
        {
            "id": "feat-account-bound-provider-verification",
            "parentId": "req-broker",
            "title": "계좌·Bot·Cloud 실연동 검증 묶음",
            "description": "KIS·Toss·Upbit·Binance·Telegram Bot·Google Cloud 자격정보가 준비된 뒤 읽기/모의주문/회신/대사를 공급자별로 실제 검증한다.",
            "status": "planned",
            "priority": "high",
            "role": "외부 어댑터 담당 · 운영 담당",
            "sortOrder": 2116,
            "colorKey": "amber",
            "acceptanceCriteria": acceptance(
                "feat-account-bound-provider-verification",
                [
                    "비밀정보를 OS·Cloud 보안 저장소 밖에 기록하지 않는다.",
                    "읽기 전용과 모의 주문 권한을 공급자별 격리 계정에서 검증한다.",
                    "실전 주문·출금은 계속 잠그고 계정이 없으면 미연결로 표시한다.",
                ],
                [False, False, False],
            ),
        },
    ]


def normalize_flow(current: dict[str, Any]) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    nodes: list[dict[str, Any]] = []
    for node in current["userFlow"]["nodes"]:
        metadata = json.loads(node.get("metadata_json") or "{}")
        nodes.append(
            {
                "id": node["id"],
                "laneId": node["lane_id"],
                "title": node["title"],
                "description": node["description"],
                "kind": node["kind"],
                "positionX": node["position_x"],
                "positionY": node["position_y"],
                "colorKey": node.get("color_key") or "violet",
                "depth": node.get("depth"),
                "parentId": node.get("parent_id"),
                "linkedFeatureIds": json.loads(node.get("linked_feature_ids") or "[]"),
                "branchCondition": node.get("branch_condition"),
                "inputArtifacts": metadata.get("inputArtifacts", []),
                "outputArtifacts": metadata.get("outputArtifacts", []),
                "methods": metadata.get("methods", []),
                "validation": metadata.get("validation", ""),
                "failureHandling": metadata.get("failureHandling", ""),
                "codePaths": metadata.get("codePaths", []),
                "testPaths": metadata.get("testPaths", []),
                "completionCriteria": metadata.get("completionCriteria", ""),
                "isCompleted": bool(metadata.get("isCompleted", False)),
            }
        )
    edges = [
        {
            "id": edge["id"],
            "sourceNodeId": edge["source_node_id"],
            "targetNodeId": edge["target_node_id"],
        }
        for edge in current["userFlow"]["edges"]
    ]
    return nodes, edges


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("projectstudio_root", type=Path)
    args = parser.parse_args()
    api = load_api(args.projectstudio_root)
    database = api.default_database_path()
    with api.connect_database(database, writable=True) as connection:
        current = api.get_project(connection, PROJECT_ID)
        desired_features = feature_nodes()
        replacements = {feature["id"]: feature for feature in desired_features}
        existing_by_id = {feature["id"]: feature for feature in current["features"]}
        comparison_keys = (
            "parentId",
            "title",
            "description",
            "status",
            "priority",
            "role",
            "sortOrder",
            "colorKey",
            "acceptanceCriteria",
        )
        features_changed = any(
            feature["id"] not in existing_by_id
            or any(
                existing_by_id[feature["id"]].get(key) != feature.get(key)
                for key in comparison_keys
            )
            for feature in desired_features
        )
        features = [replacements.pop(feature["id"], feature) for feature in current["features"]]
        features.extend(replacements.values())
        markdown = current["project"]["prd_markdown"]
        section_missing = SECTION_MARKER not in markdown
        runtime_section_missing = RUNTIME_SECTION_MARKER not in markdown
        if section_missing:
            markdown += f"""

## {SECTION_MARKER} (2026-08-25)
- 즉시 개발: 백테스트 경험적 부트스트랩, 동일 통화 포트폴리오 VaR·CVaR·상관·스트레스, 전략 쿨다운·손실 보호 평가 계약을 완료했다.
- 즉시 후속: 보호 평가를 승인된 위험 정책과 섀도우·내부 모의주문 후보 경로에 연결하고 잠금 사건을 저장한다.
- Codex 연결: 레퍼런스·뉴스·커뮤니티 근거 구조화, 역할 소견과 Telegram 원격 작업 회신을 담당하되 정책 적용·주문 권한은 주지 않는다.
- 계좌 연결: KIS·Toss·Upbit·Binance와 Telegram Bot·Google Cloud는 실제 자격정보가 준비된 뒤 별도 검증한다. 실전 주문·출금은 계속 잠근다.
- 모델 연구: Amazon Chronos와 Google TimesFM은 Apache-2.0 후보로 기록하고 공통 확률 예측 어댑터를 계획한다. Python·가중치는 승인 전 설치하지 않는다.
- LEAN·Qlib·Freqtrade·NautilusTrader의 구조와 검증 관점을 참고했으며 외부 코드·전략·성과 수치는 복사하지 않았다.
"""
        if runtime_section_missing:
            markdown += f"""

## {RUNTIME_SECTION_MARKER} (2026-08-25)
- 사용자 승인 위험정책에 선택형 전략 보호정책을 포함하고 기존 정책 JSON은 `protection` 없음으로 호환한다.
- 위험정책 추천 백테스트에서 쿨다운·연속손실·낙폭·종목 저수익 트리거를 함께 기록한다.
- 활성 정책은 내부 모의원장의 종료 거래 손익을 재구성해 신규 매수 후보 생성·승인 직전에 검사한다. 매도와 실전 주문은 각각 위험 축소 허용·전송 금지 상태다.
- 허용·차단 결정은 SQLite에 저장하고 재시작 후 이력 조회가 가능하다.
- 포트폴리오 위험 입력·결과를 불변 스냅샷으로 저장·조회하며 혼합 통화를 임의 환산하지 않는다.
- 연구원 백테스트 화면에 부트스트랩 5~95% 범위와 손실·자본 50% 이하 경로 확률을 표시한다.
- 남은 작업은 운영 화면의 보호 이력, 원장 기반 포트폴리오 수익률 자동 구성, 종료 사유 구조화다.
"""
        if not features_changed and not section_missing and not runtime_section_missing:
            print(
                json.dumps(
                    {
                        "projectId": PROJECT_ID,
                        "committed": False,
                        "message": "동일한 레퍼런스 작업군 기획이 이미 반영되어 있습니다.",
                    },
                    ensure_ascii=False,
                    indent=2,
                )
            )
            return
        nodes, edges = normalize_flow(current)
        bundle = {
            "schemaVersion": 1,
            "projectId": PROJECT_ID,
            "expectedPrdRevisionNumber": current["project"]["revision_number"],
            "prd": {"title": current["project"]["prd_title"], "markdown": markdown},
            "features": features,
            "userFlow": {"nodes": nodes, "edges": edges},
        }
        print(json.dumps(api.apply_bundle(connection, database, bundle, commit=True), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
