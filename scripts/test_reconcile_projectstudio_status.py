import unittest
from pathlib import Path

import reconcile_projectstudio_status as reconciliation


class MeetingFlowReconciliationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.nodes = reconciliation.meeting_flow_nodes()
        self.edges = reconciliation.meeting_flow_edges()
        self.by_id = {node["id"]: node for node in self.nodes}

    def test_all_meeting_nodes_are_reachable_from_phase(self) -> None:
        self.assertEqual(len(self.by_id), len(self.nodes))
        graph = {node_id: [] for node_id in self.by_id}
        for edge in self.edges:
            self.assertIn(edge["sourceNodeId"], self.by_id)
            self.assertIn(edge["targetNodeId"], self.by_id)
            graph[edge["sourceNodeId"]].append(edge["targetNodeId"])

        reachable: set[str] = set()
        pending = ["flow-meeting-analysis-cycle-phase"]
        while pending:
            node_id = pending.pop()
            if node_id in reachable:
                continue
            reachable.add(node_id)
            pending.extend(graph[node_id])

        self.assertEqual(reachable, set(self.by_id))

    def test_only_live_candidate_and_shadow_portion_remains_unchecked(self) -> None:
        pending_ids = {
            "flow-meeting-analysis-cycle-phase",
            "flow-meeting-analysis-cycle-7",
            "flow-meeting-analysis-cycle-shadow",
        }
        for node_id in pending_ids:
            self.assertFalse(self.by_id[node_id]["isCompleted"], node_id)
        for node_id in {
            "flow-meeting-analysis-cycle-approval",
            "flow-meeting-analysis-cycle-execution",
            "flow-meeting-analysis-cycle-ledger",
        }:
            self.assertTrue(self.by_id[node_id]["isCompleted"], node_id)

    def test_failure_branches_are_explicit(self) -> None:
        branch_ids = {
            "flow-meeting-analysis-cycle-symbol-error",
            "flow-meeting-analysis-cycle-provider-error",
            "flow-meeting-analysis-cycle-partial-failure",
            "flow-meeting-analysis-cycle-codex-recovery",
            "flow-meeting-analysis-cycle-risk-rejected",
            "flow-meeting-analysis-cycle-duplicate",
        }
        self.assertTrue(branch_ids.issubset(self.by_id))
        for node_id in branch_ids:
            self.assertTrue(self.by_id[node_id]["branchCondition"], node_id)

    def test_declared_evidence_paths_exist(self) -> None:
        repository = Path(__file__).resolve().parents[1]
        for node in self.nodes:
            for relative_path in node["codePaths"] + node["testPaths"]:
                self.assertTrue((repository / relative_path).is_file(), relative_path)


if __name__ == "__main__":
    unittest.main()
