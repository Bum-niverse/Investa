"""토스 시장 랭킹 기반 다단계 후보 탐색 기획을 ProjectStudio에 멱등 반영한다."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path


PROJECT_ID = "36e87491-74a8-48ca-a7b8-30fa6ccea131"
FEATURE_ID = "feat-toss-ranked-market-screener"
FLOW_NODE_ID = "flow-daily-market-screener"
SECTION_MARKER = "토스 시장 랭킹 기반 다단계 후보 탐색"


def load_api(root: Path):
    source = root / "scripts" / "projectstudio_api.py"
    spec = importlib.util.spec_from_file_location("projectstudio_api", source)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"ProjectStudio API를 불러오지 못했습니다: {source}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def normalize_flow(current):
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


def desired_feature():
    criteria = [
        "국장·미장 시장 거래대금·거래량과 preset별 급등락 랭킹을 읽기 전용으로 합쳐 1차 후보를 만든다.",
        "상위 12종목에만 완료 수정주가 일봉을 요청하고 기존 결정론적 스크리너로 최대 5개 후보를 만든다.",
        "랭킹 시각·관측 시각·제외 수·부분 실패를 보존하고 조건 미달을 자동 완화하지 않는다.",
        "후보 탐색은 호가 검증을 유예한 분석 입력이며 추천·백테스트·주문 승인으로 자동 승격하지 않는다.",
        "사용자가 활성화하지 않은 업종·테마 집중 한도를 위반 판정에 사용하지 않는다.",
    ]
    return {
        "id": FEATURE_ID, "parentId": "req-screening", "title": "토스 시장 랭킹 기반 다단계 후보 탐색",
        "description": "시장 전체 랭킹을 저비용 1차 필터로 사용하고 제한된 후보에만 일봉 기술 검토를 실행한다.",
        "status": "done", "priority": "high", "role": "리서치부 · 데이터 품질 담당 · 리스크관리부", "sortOrder": 636, "colorKey": "green",
        "acceptanceCriteria": [{"id": f"{FEATURE_ID}-ac-{index + 1}", "description": item, "isMet": True, "sortOrder": index} for index, item in enumerate(criteria)],
    }


def desired_node():
    return {
        "id": FLOW_NODE_ID, "laneId": "lane-daily", "title": "시장 후보 탐색", "description": "토스 시장 랭킹으로 1차 후보를 줄이고 완료 일봉 규칙으로 분석 후보를 확정한다.",
        "kind": "action", "positionX": 730, "positionY": 1337, "colorKey": "green", "depth": None, "parentId": None,
        "linkedFeatureIds": [FEATURE_ID], "branchCondition": "사용자가 국장 또는 미장과 균형·추세·반전 관찰 기준을 선택함",
        "inputArtifacts": ["토스 공식 시장 랭킹", "활성 종목 카탈로그", "완료 수정주가 일봉"], "outputArtifacts": ["근거가 있는 분석 후보", "제외·부분 실패 요약"],
        "methods": ["coarse/fine 다단계 필터", "호출 예산 12종목", "PIT 완료봉", "결정론적 제외 사유"],
        "validation": "실주문 비활성, 후보·제외·오류 합계와 관측 시각을 확인하고 미래 봉이 섞이지 않아야 한다.",
        "failureHandling": "랭킹 전체 실패는 닫고, 종목별 일봉 실패는 부분 실패로 표시하며 조건을 자동 완화하지 않는다.",
        "codePaths": ["src-tauri/src/market_data.rs", "src-tauri/src/screening.rs", "src/AnalysisWorkspace.tsx"],
        "testPaths": ["src-tauri/src/market_data.rs", "src-tauri/src/screening.rs"],
        "completionCriteria": "최대 5개 후보가 탐색 근거와 함께 표시되고 선택 시 분석 회의 안건만 작성된다.", "isCompleted": True,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("projectstudio_root", type=Path)
    args = parser.parse_args()
    api = load_api(args.projectstudio_root)
    database = api.default_database_path()
    with api.connect_database(database, writable=True) as connection:
        current = api.get_project(connection, PROJECT_ID)
        feature = desired_feature()
        features = [feature if item["id"] == FEATURE_ID else item for item in current["features"]]
        if not any(item["id"] == FEATURE_ID for item in features):
            features.append(feature)
        nodes, edges = normalize_flow(current)
        node = desired_node()
        nodes = [node if item["id"] == FLOW_NODE_ID else item for item in nodes]
        if not any(item["id"] == FLOW_NODE_ID for item in nodes):
            nodes.append(node)
        edges = [item for item in edges if item["id"] != "edge-daily-1"]
        desired_edges = [
            {"id": "edge-daily-1-screener", "sourceNodeId": "flow-daily-1", "targetNodeId": FLOW_NODE_ID},
            {"id": "edge-daily-screener-2", "sourceNodeId": FLOW_NODE_ID, "targetNodeId": "flow-daily-2"},
        ]
        desired_edge_ids = {item["id"] for item in desired_edges}
        edges = [item for item in edges if item["id"] not in desired_edge_ids] + desired_edges
        markdown = current["project"]["prd_markdown"]
        if SECTION_MARKER not in markdown:
            markdown += f"""

## {SECTION_MARKER} (2026-09-03)
- 국장·미장의 시장 거래대금·거래량과 preset별 급등락 랭킹을 1차 유니버스로 사용하고 상위 12종목에만 완료 수정주가 일봉을 요청한다.
- 균형·추세·반전 관찰 규칙은 버전형 결정론적 스크리너로 적용하며 최대 5개 후보, 조건 제외와 종목별 부분 실패를 함께 표시한다.
- 현재 활성 종목 기반 결과는 과거 유니버스 백테스트가 아니며 호가 스프레드는 주문 전 별도 게이트에서 확인한다. 후보는 분석 안건으로만 연결한다.
- 업종·테마 집중은 사용자가 명시적 한도를 활성화한 경우에만 위반 판단한다.
"""
        existing_feature = next((item for item in current["features"] if item["id"] == FEATURE_ID), None)
        existing_nodes, existing_edges = normalize_flow(current)
        existing_node = next((item for item in existing_nodes if item["id"] == FLOW_NODE_ID), None)
        existing_edge_map = {item["id"]: item for item in existing_edges}
        if existing_feature == feature and existing_node == node and all(existing_edge_map.get(item["id"]) == item for item in desired_edges) and "edge-daily-1" not in existing_edge_map and SECTION_MARKER in current["project"]["prd_markdown"]:
            print(json.dumps({"projectId": PROJECT_ID, "committed": False, "message": "동일한 시장 후보 탐색 기획이 이미 반영되어 있습니다."}, ensure_ascii=False, indent=2))
            return
        bundle = {
            "schemaVersion": 1, "projectId": PROJECT_ID, "expectedPrdRevisionNumber": current["project"]["revision_number"],
            "prd": {"title": current["project"]["prd_title"], "markdown": markdown}, "features": features, "userFlow": {"nodes": nodes, "edges": edges},
        }
        api.validate_bundle(bundle)
        print(json.dumps(api.apply_bundle(connection, database, bundle, commit=True), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
