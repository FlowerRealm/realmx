import sys
import unittest
from pathlib import Path


SCRIPTS_DIR = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPTS_DIR))

import gh_pr_watch


class GhPrWatchTests(unittest.TestCase):
    def test_review_submission_surface_rules_ignore_empty_approval(self):
        approved = gh_pr_watch.normalize_reviews(
            [
                {
                    "id": 1,
                    "state": "APPROVED",
                    "user": {"login": "su8-reviewer[bot]"},
                    "body": "",
                    "html_url": "https://example.com/approved",
                }
            ]
        )[0]
        self.assertEqual(approved["state"], "APPROVED")
        self.assertFalse(gh_pr_watch.should_surface_review_submission(approved))

        approved_with_body = gh_pr_watch.normalize_reviews(
            [
                {
                    "id": 2,
                    "state": "APPROVED",
                    "user": {"login": "su8-reviewer[bot]"},
                    "body": "Looks good, but please double-check the watcher state logic.",
                    "html_url": "https://example.com/approved-with-body",
                }
            ]
        )[0]
        self.assertTrue(gh_pr_watch.should_surface_review_submission(approved_with_body))

        changes_requested = gh_pr_watch.normalize_reviews(
            [
                {
                    "id": 3,
                    "state": "CHANGES_REQUESTED",
                    "user": {"login": "su8-reviewer[bot]"},
                    "body": "",
                    "html_url": "https://example.com/changes-requested",
                }
            ]
        )[0]
        self.assertTrue(gh_pr_watch.should_surface_review_submission(changes_requested))

    def test_extract_unresolved_review_comments_uses_latest_trusted_reviewer_comment(self):
        review_threads = [
            {
                "id": "THREAD_1",
                "isResolved": False,
                "comments": {
                    "nodes": [
                        {
                            "databaseId": 11,
                            "author": {"login": "reviewer-member"},
                            "authorAssociation": "MEMBER",
                            "body": "Please keep this unresolved until the stop condition is fixed.",
                            "createdAt": "2026-03-14T12:00:00Z",
                            "path": ".realmx/skills/babysit-pr/scripts/gh_pr_watch.py",
                            "line": 560,
                            "url": "https://example.com/review-comment-11",
                        },
                        {
                            "databaseId": 12,
                            "author": {"login": "FlowerRealm"},
                            "authorAssociation": "OWNER",
                            "body": "Looking into it.",
                            "createdAt": "2026-03-14T12:05:00Z",
                            "path": ".realmx/skills/babysit-pr/scripts/gh_pr_watch.py",
                            "line": 560,
                            "url": "https://example.com/review-comment-12",
                        },
                    ]
                },
            },
            {
                "id": "THREAD_2",
                "isResolved": True,
                "comments": {
                    "nodes": [
                        {
                            "databaseId": 21,
                            "author": {"login": "reviewer-member"},
                            "authorAssociation": "MEMBER",
                            "body": "This one is already resolved.",
                            "createdAt": "2026-03-14T12:10:00Z",
                            "path": ".realmx/skills/babysit-pr/scripts/gh_pr_watch.py",
                            "line": 600,
                            "url": "https://example.com/review-comment-21",
                        }
                    ]
                },
            },
        ]

        pending = gh_pr_watch.extract_unresolved_review_comments(
            review_threads,
            authenticated_login="FlowerRealm",
        )

        self.assertEqual(
            pending,
            [
                {
                    "kind": "review_comment",
                    "id": "11",
                    "thread_id": "THREAD_1",
                    "author": "reviewer-member",
                    "author_association": "MEMBER",
                    "created_at": "2026-03-14T12:00:00Z",
                    "body": "Please keep this unresolved until the stop condition is fixed.",
                    "path": ".realmx/skills/babysit-pr/scripts/gh_pr_watch.py",
                    "line": 560,
                    "url": "https://example.com/review-comment-11",
                }
            ],
        )

    def test_ready_to_merge_waits_for_blocking_review_items(self):
        pr = {
            "closed": False,
            "merged": False,
            "mergeable": "MERGEABLE",
            "merge_state_status": "CLEAN",
            "review_decision": "",
        }
        checks_summary = {
            "all_terminal": True,
            "failed_count": 0,
            "pending_count": 0,
        }
        blocking_review_items = [
            {
                "kind": "review_comment",
                "id": "11",
                "thread_id": "THREAD_1",
            }
        ]

        self.assertFalse(
            gh_pr_watch.is_pr_ready_to_merge(pr, checks_summary, blocking_review_items)
        )
        self.assertEqual(
            gh_pr_watch.recommend_actions(
                pr,
                checks_summary,
                [],
                [],
                blocking_review_items,
                0,
                3,
            ),
            ["process_review_comment"],
        )

    def test_closed_pr_stops_without_review_processing_actions(self):
        pr = {
            "closed": True,
            "merged": False,
            "mergeable": "UNKNOWN",
            "merge_state_status": "UNKNOWN",
            "review_decision": "CHANGES_REQUESTED",
        }
        checks_summary = {
            "all_terminal": True,
            "failed_count": 0,
            "pending_count": 0,
        }

        self.assertEqual(
            gh_pr_watch.recommend_actions(
                pr,
                checks_summary,
                [],
                [
                    {
                        "kind": "review_comment",
                        "id": "99",
                        "thread_id": "THREAD_CLOSED",
                    }
                ],
                [
                    {
                        "kind": "review_comment",
                        "id": "99",
                        "thread_id": "THREAD_CLOSED",
                    }
                ],
                0,
                3,
            ),
            ["stop_pr_closed"],
        )


if __name__ == "__main__":
    unittest.main()
