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
SECTION_MARKER = "기술적 분석가 차트 주석 리포트"


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
        desired = feature()
        feature_by_id = {item["id"]: item for item in current["features"]}
        features_changed = feature_by_id.get(FEATURE_ID) != desired
        features = [desired if item["id"] == FEATURE_ID else item for item in current["features"]]
        if FEATURE_ID not in feature_by_id:
            features.append(desired)

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

        if not features_changed and had_flow and had_section:
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
