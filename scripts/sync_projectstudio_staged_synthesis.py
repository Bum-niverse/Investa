"""장문 회의 단계형 종합과 부분 실패 보존을 ProjectStudio에 멱등 반영한다."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path


PROJECT_ID = "36e87491-74a8-48ca-a7b8-30fa6ccea131"
FEATURE_ID = "feat-staged-meeting-synthesis"
FLOW_NODE_ID = "flow-meeting-staged-synthesis"
SECTION_MARKER = "장문 회의 단계형 종합과 부분 실패 보존"


def load_api(root: Path):
    source = root / "scripts" / "projectstudio_api.py"
    spec = importlib.util.spec_from_file_location("projectstudio_api", source)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"ProjectStudio API를 불러오지 못했습니다: {source}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def desired_feature():
    criteria = [
        "직원 RoleReport와 전체 부서 보고 원문은 로컬 분석 기록에 그대로 보존한다.",
        "본부장 입력은 직원→부서장 중간 산출물과 evidenceId 중복 제거 근거로 결정론적으로 조립한다.",
        "입력 근거 발생 수·고유 수·포함 수·제외 수·최종 문자 수를 synthesisTrace에 저장한다.",
        "최종 종합이 길이·timeout·계약 검증으로 실패해도 완료 부서 보고와 실패 사유를 hold 기록으로 저장한다.",
        "계좌 식별자·자격정보는 종합 입력과 분석 기록에 추가하지 않고 SHADOW ONLY를 유지한다.",
    ]
    return {
        "id": FEATURE_ID,
        "parentId": "feat-complete-investment-analysis-report",
        "title": "장문 회의 단계형 종합과 부분 실패 보존",
        "description": "직원 원문을 잃지 않으면서 부서별 중간 종합과 중복 제거 근거로 본부장 입력을 안전하게 구성하고 실패 결과도 기록한다.",
        "status": "done",
        "priority": "high",
        "role": "AI 투자본부장 · 데이터 품질 담당 · 보안 검토자",
        "sortOrder": 639,
        "colorKey": "green",
        "acceptanceCriteria": [
            {"id": f"{FEATURE_ID}-ac-{index + 1}", "description": item, "isMet": True, "sortOrder": index}
            for index, item in enumerate(criteria)
        ],
    }


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
        node = {
            "id": FLOW_NODE_ID, "laneId": "lane-meeting-analysis-cycle", "title": "단계형 장문 종합", "description": "직원 원문은 보존하고 부서별 중간 결과와 중복 제거 근거만 본부장에게 전달한다.",
            "kind": "action", "positionX": 1240, "positionY": 4600, "colorKey": "green", "depth": None, "parentId": None,
            "linkedFeatureIds": [FEATURE_ID, "feat-complete-investment-analysis-report"], "branchCondition": "모든 소집 부서 보고 완료",
            "inputArtifacts": ["RoleReport", "DepartmentReport", "근거 메타데이터", "익명화 포트폴리오 맥락"], "outputArtifacts": ["MeetingSynthesis", "synthesisTrace", "부분 실패 분석 기록"],
            "methods": ["evidenceId 중복 제거", "부서별 단계형 압축", "44,000자 종합 입력 예산", "실패 시 hold 기록"],
            "validation": "대형 보고 입력이 48,000자 하드 상한보다 4,000자 여유 안에서 생성되고 원본 부서 보고는 저장 기록에 남는다.",
            "failureHandling": "최종 종합 실패 시 완료 부서 요약과 원문, 실패 사유, 재실행 조건을 hold 기록으로 남긴다.",
            "codePaths": ["src/meetingSynthesis.ts", "src/App.tsx"], "testPaths": ["scripts/meetingSynthesis.test.ts"],
            "completionCriteria": "중복 근거가 많은 전체 포트폴리오 회의도 종합 입력 상한을 지키며 실패 시 부분 결과를 잃지 않는다.", "isCompleted": True,
        }
        nodes = [node if item["id"] == FLOW_NODE_ID else item for item in nodes]
        if not any(item["id"] == FLOW_NODE_ID for item in nodes):
            nodes.append(node)
        edge = {"id": "edge-meeting-evidence-staged-synthesis", "sourceNodeId": "flow-meeting-analysis-cycle-evidence", "targetNodeId": FLOW_NODE_ID}
        if not any(item["id"] == edge["id"] for item in edges):
            edges.append(edge)
        markdown = current["project"]["prd_markdown"]
        if SECTION_MARKER not in markdown:
            markdown += f"""

## {SECTION_MARKER} (2026-09-04)
- 직원별 RoleReport와 부서 보고 원문은 로컬 분석 기록에 보존하고, 본부장에게는 부서별 중간 산출물과 evidenceId 기준으로 중복 제거한 근거만 전달한다.
- 종합 입력은 48,000자 하드 상한보다 4,000자 작은 예산에서 조립하며 근거 발생·고유·포함·제외 수와 최종 문자 수를 기록한다.
- 길이 초과, timeout 또는 구조화 계약 실패가 발생해도 완료된 부서 보고와 실패 사유를 분석 보관함의 hold 기록으로 남긴다.
- 외부 자료는 비신뢰 입력으로 취급하고 계좌 식별자·자격정보·실주문 권한을 추가하지 않는다.
"""
        existing_feature = next((item for item in current["features"] if item["id"] == FEATURE_ID), None)
        existing_nodes = {item["id"]: item for item in normalize_flow(current)[0]}
        if existing_feature == feature and existing_nodes.get(FLOW_NODE_ID) == node and SECTION_MARKER in current["project"]["prd_markdown"]:
            print(json.dumps({"projectId": PROJECT_ID, "committed": False, "message": "동일한 단계형 종합 기획이 이미 반영되어 있습니다."}, ensure_ascii=False, indent=2))
            return
        bundle = {
            "schemaVersion": 1, "projectId": PROJECT_ID, "expectedPrdRevisionNumber": current["project"]["revision_number"],
            "prd": {"title": current["project"]["prd_title"], "markdown": markdown}, "features": features, "userFlow": {"nodes": nodes, "edges": edges},
        }
        api.validate_bundle(bundle)
        print(json.dumps(api.apply_bundle(connection, database, bundle, commit=True), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
