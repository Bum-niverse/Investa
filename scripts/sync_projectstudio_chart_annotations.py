"""기술적 분석가의 차트 주석 리포트 기능을 ProjectStudio Investa 기획에 멱등 반영한다."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path
from types import ModuleType
from typing import Any


PROJECT_ID = "36e87491-74a8-48ca-a7b8-30fa6ccea131"
FEATURE_ID = "feat-technical-chart-annotation-report"
FLOW_NODE_ID = "flow-analysis-vault-6"
ASSET_FEATURE_ID = "feat-cross-asset-chart-annotation-contracts"
ASSET_FLOW_NODE_ID = "flow-analysis-vault-7"
SECTION_MARKER = "기술적 분석가 차트 주석 리포트"
ASSET_SECTION_MARKER = "자산군별 차트 주석 계약"


def load_module(path: Path, name: str) -> ModuleType:
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"모듈을 불러오지 못했습니다: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def feature() -> dict[str, Any]:
    return {
        "id": FEATURE_ID,
        "parentId": "role-technical-analyst",
        "title": "기술적 분석 차트 주석 리포트",
        "description": "기술적 분석가가 받은 동일 시점 OHLCV를 불변 분석 기록에 보존하고 관측 고·저점, 저점 연결선과 최근 가격 범위를 차트 위에 표시한다. 선은 미래 예측이나 주문 신호로 사용하지 않는다.",
        "status": "done",
        "priority": "high",
        "role": "기술적 분석가",
        "sortOrder": 631,
        "colorKey": "green",
        "acceptanceCriteria": [
            {"id": "ac-technical-chart-same-snapshot", "description": "기술적 분석가에게 제공된 동일 snapshot ID와 완료 봉 최대 120개를 분석 기록에 함께 보존한다.", "isMet": True, "sortOrder": 0},
            {"id": "ac-technical-chart-deterministic-lines", "description": "최근 관측 고·저점 수평선, 시간 구간 저점 연결선과 최근 20봉 가격 범위를 실제 OHLCV 좌표로 계산한다.", "isMet": True, "sortOrder": 1},
            {"id": "ac-technical-chart-immutable-render", "description": "분석 보관함에서 저장된 캔들과 선을 다시 표시하고 현재 시세나 수동 차트 그림으로 과거 기록을 바꾸지 않는다.", "isMet": True, "sortOrder": 2},
            {"id": "ac-technical-chart-boundary", "description": "완료 봉 20개 미만이면 차트를 만들지 않고, 표시선을 예측·주문 신호가 아닌 시각 보조 자료로 안내한다.", "isMet": True, "sortOrder": 3},
        ],
    }


def asset_feature() -> dict[str, Any]:
    return {
        "id": ASSET_FEATURE_ID,
        "parentId": FEATURE_ID,
        "title": "코인·증권선물·코인선물 자산별 선긋기",
        "description": "공통 PIT 완료봉 계약 위에서 코인 현물의 24시간 구조, 증권선물의 정산가·만기·롤오버, 코인 무기한선물의 마크·지수·펀딩 근거를 서로 다른 선 규칙으로 보존한다.",
        "status": "in_progress",
        "priority": "high",
        "role": "기술적 분석가 · 파생·펀딩 담당 · 시장데이터 엔지니어",
        "sortOrder": 632,
        "colorKey": "violet",
        "acceptanceCriteria": [
            {"id": "ac-cross-chart-pit-contract", "description": "완료·공개·수집 시각과 중복·겹침을 검사하는 공통 PIT 봉 계약을 적용한다.", "isMet": True, "sortOrder": 0},
            {"id": "ac-cross-chart-securities-futures", "description": "증권선물은 현 계약 안에서만 추세를 계산하고 공식 정산가와 롤 경계를 별도 표시한다.", "isMet": True, "sortOrder": 1},
            {"id": "ac-cross-chart-crypto", "description": "코인 현물은 24시간·누락 봉을, 무기한선물은 마크·지수·펀딩을 분리하고 PIT 누수 테스트를 통과한다.", "isMet": True, "sortOrder": 2},
            {"id": "ac-cross-chart-crypto-provider-adapters", "description": "Upbit 현물과 Binance USDⓈ-M 무기한선물 공개 snapshot이 역할 분석과 분석 보관함 계약까지 실제 왕복한다.", "isMet": True, "sortOrder": 3},
            {"id": "ac-cross-chart-securities-provider-adapter", "description": "KIS 공식 국내선물 계약별 일봉 어댑터가 계약코드·세션·PIT 시각을 보존한다. 자격정보 환경의 실제 왕복은 별도 검증한다.", "isMet": False, "sortOrder": 4},
        ],
    }


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
    api = load_module(args.projectstudio_root / "scripts" / "projectstudio_api.py", "projectstudio_api")
    database = api.default_database_path()
    with api.connect_database(database, writable=True) as connection:
        current = api.get_project(connection, PROJECT_ID)
        desired_items = [feature(), asset_feature()]
        feature_by_id = {item["id"]: item for item in current["features"]}
        desired_by_id = {item["id"]: item for item in desired_items}
        features_changed = any(feature_by_id.get(item["id"]) != item for item in desired_items)
        features = [desired_by_id.get(item["id"], item) for item in current["features"]]
        for item in desired_items:
            if item["id"] not in feature_by_id:
                features.append(item)

        nodes, edges = normalize_flow(current)
        had_flow = any(node["id"] == FLOW_NODE_ID for node in nodes)
        if not had_flow:
            nodes.append({
                "id": FLOW_NODE_ID, "laneId": "lane-analysis-vault", "title": "선 표시 차트 근거 확인",
                "description": "기술적 분석가 기록에서 동일 시점 캔들, 관측 고·저점 수평선, 저점 연결선과 최근 가격 범위를 확인한다.",
                "kind": "result", "positionX": 1570, "positionY": 2380, "colorKey": "green", "depth": 6,
                "parentId": "flow-analysis-vault-5", "linkedFeatureIds": [FEATURE_ID], "branchCondition": None,
                "inputArtifacts": ["기술적 분석가 RoleReport", "AnalysisSnapshot 완료 봉"], "outputArtifacts": ["불변 SVG 차트 근거"],
                "methods": ["결정론적 고·저점·가격 범위 주석"], "validation": "snapshot ID와 차트 봉·선 좌표가 저장 기록에서 재현된다.",
                "failureHandling": "완료 봉이 20개 미만이면 차트를 만들지 않고 텍스트 소견과 근거 공백만 유지한다.",
                "codePaths": ["src/technicalChartEvidence.ts", "src/TechnicalChartEvidenceView.tsx", "src/AnalysisWorkspace.tsx"],
                "testPaths": ["scripts/technicalChartEvidence.test.ts"], "completionCriteria": "기술적 분석가의 새 소견 기록을 열면 같은 시점의 선 표시 차트를 확인할 수 있다.",
                "isCompleted": True,
            })
            edges.append({"id": "edge-analysis-vault-5-6", "sourceNodeId": "flow-analysis-vault-5", "targetNodeId": FLOW_NODE_ID})
        had_asset_flow = any(node["id"] == ASSET_FLOW_NODE_ID for node in nodes)
        if not had_asset_flow:
            nodes.append({
                "id": ASSET_FLOW_NODE_ID, "laneId": "lane-analysis-vault", "title": "자산별 선 근거 검증",
                "description": "코인 현물·증권선물·코인 무기한선물의 PIT 봉 계약과 서로 다른 가격 기준선을 검증한다.",
                "kind": "decision", "positionX": 1830, "positionY": 2380, "colorKey": "violet", "depth": 7,
                "parentId": FLOW_NODE_ID, "linkedFeatureIds": [ASSET_FEATURE_ID], "branchCondition": None,
                "inputArtifacts": ["자산군", "PIT 완료 봉", "계약·정산·마크·지수·펀딩 메타데이터"], "outputArtifacts": ["자산별 불변 차트 근거"],
                "methods": ["PIT 누수 차단", "롤 경계 분리", "마크·지수·펀딩 분리"], "validation": "미래 공개 봉과 필수 자산 메타데이터 누락은 차트 생성을 차단한다.",
                "failureHandling": "공식 공급자 메타데이터가 없으면 값을 추정하지 않고 차트 근거를 생성하지 않는다.",
                "codePaths": ["src/technicalChartEvidence.ts", "src/TechnicalChartEvidenceView.tsx"], "testPaths": ["scripts/technicalChartEvidence.test.ts"],
                "completionCriteria": "자산별 계약 테스트는 통과하고 공식 공급자 왕복은 별도 미완료로 표시된다.", "isCompleted": False,
            })
            edges.append({"id": "edge-analysis-vault-6-7", "sourceNodeId": FLOW_NODE_ID, "targetNodeId": ASSET_FLOW_NODE_ID})
        else:
            asset_node = next(node for node in nodes if node["id"] == ASSET_FLOW_NODE_ID)
            asset_node["description"] = "Upbit 코인 현물·Binance 코인 무기한선물 공개 snapshot 왕복과 KIS 공식 국내선물 계약별 일봉 어댑터를 검증한다. KIS 자격정보 환경의 실제 왕복은 미완료다."
            asset_node["codePaths"] = ["src/technicalChartEvidence.ts", "src/TechnicalChartEvidenceView.tsx", "src/analysisSnapshotRouting.ts", "src-tauri/src/crypto_market.rs", "src-tauri/src/binance.rs", "src-tauri/src/kis_paper.rs"]
            asset_node["testPaths"] = ["scripts/technicalChartEvidence.test.ts", "scripts/analysisSnapshotRouting.test.ts", "src-tauri/src/crypto_market.rs", "src-tauri/src/binance.rs", "src-tauri/src/kis_paper.rs"]
            asset_node["completionCriteria"] = "Upbit·Binance 공개 snapshot 왕복과 KIS 어댑터 계약 검증은 완료됐고 KIS 실제 왕복을 완료하면 노드를 체크한다."
            asset_node["isCompleted"] = False

        markdown = current["project"]["prd_markdown"]
        had_section = SECTION_MARKER in markdown
        if not had_section:
            markdown += f"""

## {SECTION_MARKER} (2026-08-26)
- 담당자는 리서치부 기술적 분석가이며 재무·뉴스·최종 주문 판단을 대신하지 않는다.
- 기술적 분석가에게 전달된 동일 시점 OHLCV 완료 봉을 분석 기록에 함께 보존하고 최근 관측 고·저점, 구간 저점 연결선과 최근 20봉 가격 범위를 결정론적으로 표시한다.
- 저장된 차트는 현재 시세와 사용자의 수동 선에 의해 바뀌지 않는 불변 근거다. 완료 봉이 20개 미만이면 차트를 만들지 않는다.
- 키움 HTS의 추세선·수평선·박스권·종목별 보존 흐름과 TradingView 공식 primitive 예제를 참고했으며 새 차트 라이브러리는 추가하지 않았다.
"""
        had_asset_section = ASSET_SECTION_MARKER in markdown
        if not had_asset_section:
            markdown += f"""

## {ASSET_SECTION_MARKER} (2026-08-27)
- 코인 현물은 24시간 연속시장과 무거래 구간의 봉 누락을 보존하며 빈 봉을 임의 생성하지 않는다.
- 증권선물은 공식 정산가·계약코드·만기·롤오버를 보존하고 추세선이 계약 경계를 가로지르지 못하게 한다.
- 코인 무기한선물은 체결가, 마크가격, 지수가격과 펀딩 시점을 서로 다른 근거로 표시한다.
- 분석 기준 시각 뒤에 완료·공개·수집된 봉은 PIT 누수로 차단한다. Upbit 현물과 Binance USDⓈ-M 공개 snapshot 왕복은 완료됐으며 공식 증권선물 공급자 연결만 미완료다.
"""
        else:
            markdown = markdown.replace(
                "Upbit 현물과 Binance USDⓈ-M 공개 snapshot 왕복은 완료됐으며 공식 증권선물 공급자 연결만 미완료다.",
                "Upbit 현물과 Binance USDⓈ-M 공개 snapshot 왕복은 완료됐다. KIS 공식 국내선물 계약별 일봉 어댑터는 구현됐으며 자격정보 환경의 실제 왕복만 미완료다.",
            )

        if not features_changed and had_flow and had_asset_flow and had_section and had_asset_section:
            print(json.dumps({"projectId": PROJECT_ID, "committed": False, "message": "이미 동일한 차트 주석 기획이 반영되어 있습니다."}, ensure_ascii=False, indent=2))
            return

        bundle = {
            "schemaVersion": 1,
            "projectId": PROJECT_ID,
            "expectedPrdRevisionNumber": current["project"]["revision_number"],
            "prd": {"title": current["project"]["prd_title"], "markdown": markdown},
            "features": features,
            "userFlow": {"nodes": nodes, "edges": edges},
        }
        api.validate_bundle(bundle)
        result = api.apply_bundle(connection, database, bundle, commit=True)
        print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
