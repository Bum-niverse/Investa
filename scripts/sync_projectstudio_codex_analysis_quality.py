"""Codex 분석 품질 프로필을 ProjectStudio Investa 기능명세에 멱등 반영한다."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path


PROJECT_ID = "36e87491-74a8-48ca-a7b8-30fa6ccea131"
FEATURE_ID = "feat-codex-analysis-quality-profile"
FLOW_NODE_ID = "flow-meeting-analysis-cycle-3"
SECTION_MARKER = "Codex 분석 품질 프로필"


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
        ("catalog", "Codex model/list를 조회해 현재 계정이 실제 지원하는 모델과 reasoning effort만 사용한다."),
        ("profiles", "안건 분류는 medium, 직원·부서 분석은 high, 본부장 최종 종합은 xhigh 목표 프로필로 실행한다."),
        ("fallback", "목표 effort가 지원되지 않으면 카탈로그 안에서만 보수적으로 낮추고 존재하지 않는 조합을 보내지 않는다."),
        ("raw-evidence", "본부장 최종 종합에도 동일 기준 시각의 원본 재무·공시·뉴스·포지션 근거를 다시 전달한다."),
        ("evidence-gate", "부서 보고가 전달받지 않은 근거 ID를 사용하면 실패로 닫고 모의투자 후보 승격을 차단한다."),
        ("security", "Codex 세션의 읽기 전용·네트워크 차단·승인 불가·실주문 금지 경계를 유지한다."),
    ]
    return {
        "id": FEATURE_ID,
        "parentId": "feat-agents-trace",
        "title": "Codex 분석 품질 프로필 및 원본 근거 종합",
        "description": "계정 지원 모델을 동적으로 검증하고 분석 단계별 추론 강도와 근거 무결성 게이트를 적용한다.",
        "status": "done",
        "priority": "high",
        "role": "AI 오케스트레이션 담당 · 데이터 품질 담당 · 보안 담당",
        "sortOrder": 634,
        "colorKey": "green",
        "acceptanceCriteria": [
            {"id": f"ac-codex-quality-{suffix}", "description": description, "isMet": True, "sortOrder": index}
            for index, (suffix, description) in enumerate(criteria)
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
        desired = desired_feature()
        features = [desired if item["id"] == FEATURE_ID else item for item in current["features"]]
        if not any(item["id"] == FEATURE_ID for item in features):
            features.append(desired)
        nodes, edges = normalize_flow(current)
        node = next(item for item in nodes if item["id"] == FLOW_NODE_ID)
        node["linkedFeatureIds"] = list(dict.fromkeys([*node["linkedFeatureIds"], FEATURE_ID]))
        node["methods"] = list(dict.fromkeys([*node["methods"], "지원 모델 카탈로그 검증", "단계별 추론 품질 프로필", "원본 근거 재대조", "허위 근거 ID 차단"]))
        node["codePaths"] = list(dict.fromkeys([*node["codePaths"], "src-tauri/src/codex.rs", "src/App.tsx", "src/meetingEvidence.ts"]))
        node["testPaths"] = list(dict.fromkeys([*node["testPaths"], "src-tauri/src/codex.rs", "scripts/meetingEvidence.test.ts"]))

        markdown = current["project"]["prd_markdown"]
        if SECTION_MARKER not in markdown:
            markdown += f"""

## {SECTION_MARKER} (2026-08-30)
- 현재 계정의 Codex 모델 카탈로그를 조회해 지원 모델·추론 강도만 명시적으로 사용한다.
- 안건 분류 medium, 직원·부서 분석 high, 본부장 최종 종합 xhigh를 목표로 하며 미지원 강도는 지원 범위에서만 하향한다.
- 최종 종합에도 동일한 point-in-time 원본 근거를 다시 제공하고, 존재하지 않는 evidence ID를 사용한 부서 보고는 실패로 닫는다.
- 품질 프로필은 분석 깊이만 높이며 읽기 전용·네트워크 차단·실주문 금지 경계는 변경하지 않는다.
"""

        existing = next((item for item in current["features"] if item["id"] == FEATURE_ID), None)
        if existing == desired and FEATURE_ID in node["linkedFeatureIds"] and SECTION_MARKER in current["project"]["prd_markdown"]:
            print(json.dumps({"projectId": PROJECT_ID, "committed": False, "message": "동일한 Codex 품질 기획이 이미 반영되어 있습니다."}, ensure_ascii=False, indent=2))
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
        print(json.dumps(api.apply_bundle(connection, database, bundle, commit=True), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
