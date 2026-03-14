import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


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

    def test_extract_repo_from_pr_view_prefers_base_repository(self):
        self.assertEqual(
            gh_pr_watch.extract_repo_from_pr_view(
                {
                    "baseRepository": {"name": "codex"},
                    "baseRepositoryOwner": {"login": "openai"},
                    "headRepository": {"name": "realmx"},
                    "headRepositoryOwner": {"login": "FlowerRealm"},
                }
            ),
            "openai/codex",
        )

    def test_extract_unresolved_review_comments_keeps_all_unresolved_reviewer_comments(self):
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
                        {
                            "databaseId": 13,
                            "author": {"login": "reviewer-member"},
                            "authorAssociation": "MEMBER",
                            "body": "There is also an older unresolved point in the same thread.",
                            "createdAt": "2026-03-14T12:06:00Z",
                            "path": ".realmx/skills/babysit-pr/scripts/gh_pr_watch.py",
                            "line": 561,
                            "url": "https://example.com/review-comment-13",
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
                },
                {
                    "kind": "review_comment",
                    "id": "13",
                    "thread_id": "THREAD_1",
                    "author": "reviewer-member",
                    "author_association": "MEMBER",
                    "created_at": "2026-03-14T12:06:00Z",
                    "body": "There is also an older unresolved point in the same thread.",
                    "path": ".realmx/skills/babysit-pr/scripts/gh_pr_watch.py",
                    "line": 561,
                    "url": "https://example.com/review-comment-13",
                },
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

    def test_combine_review_blocking_items_dedupes_same_comment_id_across_sources(self):
        new_review_items = [
            {
                "kind": "review_comment",
                "id": "11",
                "thread_id": "",
            }
        ]
        pending_review_comments = [
            {
                "kind": "review_comment",
                "id": "11",
                "thread_id": "THREAD_1",
            }
        ]

        self.assertEqual(
            gh_pr_watch.combine_review_blocking_items(
                new_review_items,
                pending_review_comments,
            ),
            [
                {
                    "kind": "review_comment",
                    "id": "11",
                    "thread_id": "",
                }
            ],
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

    def test_failed_checks_expose_diagnosis_and_retry_actions(self):
        pr = {
            "closed": False,
            "merged": False,
            "mergeable": "MERGEABLE",
            "merge_state_status": "CLEAN",
            "review_decision": "",
        }
        checks_summary = {
            "all_terminal": True,
            "failed_count": 1,
            "pending_count": 0,
        }

        self.assertEqual(
            gh_pr_watch.recommend_actions(
                pr,
                checks_summary,
                [{"run_id": 123}],
                [],
                [],
                0,
                3,
            ),
            ["diagnose_ci_failure", "retry_failed_checks"],
        )

    def test_run_watch_reports_updated_next_poll_seconds_after_backoff(self):
        snapshot = {
            "pr": {
                "head_sha": "abc123",
                "state": "OPEN",
                "mergeable": "MERGEABLE",
                "merge_state_status": "CLEAN",
                "review_decision": "",
            },
            "checks": {
                "all_terminal": True,
                "failed_count": 0,
                "pending_count": 0,
                "passed_count": 1,
            },
            "blocking_review_items": [],
            "actions": ["idle"],
        }
        args = SimpleNamespace(poll_seconds=30)
        events = []

        with (
            mock.patch.object(
                gh_pr_watch,
                "collect_snapshot",
                side_effect=[(snapshot, "/tmp/state"), (snapshot, "/tmp/state")],
            ),
            mock.patch.object(
                gh_pr_watch,
                "print_event",
                side_effect=lambda event, payload: events.append((event, payload)),
            ),
            mock.patch.object(
                gh_pr_watch,
                "time",
                wraps=gh_pr_watch.time,
            ) as mocked_time,
        ):
            mocked_time.sleep.side_effect = [None, RuntimeError("stop watch loop")]
            with self.assertRaisesRegex(RuntimeError, "stop watch loop"):
                gh_pr_watch.run_watch(args)

        self.assertEqual(events[0][1]["next_poll_seconds"], 30)
        self.assertEqual(events[1][1]["next_poll_seconds"], 60)

    def test_fetch_review_thread_comments_reuses_embedded_page_before_paginating(self):
        thread = {
            "id": "THREAD_1",
            "isResolved": False,
            "comments": {
                "nodes": [
                    {
                        "databaseId": 11,
                        "body": "first",
                    }
                ],
                "pageInfo": {
                    "hasNextPage": True,
                    "endCursor": "CURSOR_1",
                },
            },
        }

        with mock.patch.object(
            gh_pr_watch,
            "gh_json",
            side_effect=[
                {
                    "data": {
                        "node": {
                            "isResolved": False,
                            "comments": {
                                "nodes": [
                                    {
                                        "databaseId": 12,
                                        "body": "second",
                                    }
                                ],
                                "pageInfo": {
                                    "hasNextPage": False,
                                    "endCursor": None,
                                },
                            },
                        }
                    }
                },
            ],
        ) as mocked_gh_json:
            hydrated_thread = gh_pr_watch.fetch_review_thread_comments(thread)

        self.assertEqual(mocked_gh_json.call_count, 1)
        self.assertEqual(hydrated_thread["id"], "THREAD_1")
        self.assertFalse(hydrated_thread["isResolved"])
        self.assertEqual(
            [comment["databaseId"] for comment in hydrated_thread["comments"]["nodes"]],
            [11, 12],
        )

    def test_fetch_pending_review_comments_only_hydrates_unresolved_threads(self):
        resolved_thread = {
            "id": "THREAD_RESOLVED",
            "isResolved": True,
            "comments": {
                "nodes": [],
                "pageInfo": {
                    "hasNextPage": False,
                    "endCursor": None,
                },
            },
        }
        unresolved_thread = {
            "id": "THREAD_OPEN",
            "isResolved": False,
            "comments": {
                "nodes": [
                    {
                        "databaseId": 11,
                        "author": {"login": "reviewer-member"},
                        "authorAssociation": "MEMBER",
                        "body": "Still unresolved.",
                        "createdAt": "2026-03-14T13:20:00Z",
                        "path": ".realmx/skills/babysit-pr/scripts/gh_pr_watch.py",
                        "line": 691,
                        "url": "https://example.com/review-comment-11",
                    }
                ],
                "pageInfo": {
                    "hasNextPage": False,
                    "endCursor": None,
                },
            },
        }

        with (
            mock.patch.object(
                gh_pr_watch,
                "fetch_review_threads",
                return_value=[resolved_thread, unresolved_thread],
            ),
            mock.patch.object(
                gh_pr_watch,
                "fetch_review_thread_comments",
                side_effect=lambda thread: thread,
            ) as mocked_fetch_review_thread_comments,
        ):
            pending = gh_pr_watch.fetch_pending_review_comments(
                {"repo": "FlowerRealm/realmx", "number": 12},
                authenticated_login="FlowerRealm",
            )

        mocked_fetch_review_thread_comments.assert_called_once_with(unresolved_thread)
        self.assertEqual(
            pending,
            [
                {
                    "kind": "review_comment",
                    "id": "11",
                    "thread_id": "THREAD_OPEN",
                    "author": "reviewer-member",
                    "author_association": "MEMBER",
                    "created_at": "2026-03-14T13:20:00Z",
                    "body": "Still unresolved.",
                    "path": ".realmx/skills/babysit-pr/scripts/gh_pr_watch.py",
                    "line": 691,
                    "url": "https://example.com/review-comment-11",
                }
            ],
        )


if __name__ == "__main__":
    unittest.main()
