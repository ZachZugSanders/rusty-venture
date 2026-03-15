"""
Phase 0 — Playwright e2e tests: tier badge colour correctness.

League tiers and their expected CSS colour class substring:
  DIAMOND  / PLATINUM  → class contains "green"
  GOLD     / SILVER    → class contains "yellow"
  BRONZE               → class contains "red"

The colour functions also preserve backward-compat for rows scanned before
the league-tier rename, so we validate the legacy labels as well:
  EXEMPLARY / ESTABLISHED → green
  DEVELOPING / EMERGING   → yellow
  NASCENT                 → red  (falls through to default)

Both the **History** view and the **Repos** view expose grade badges; each is
tested independently.  All backend API responses are intercepted in conftest.py
so no running Rust server is required.
"""

import pytest
from playwright.sync_api import Page, expect

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# Mapping from grade text → expected CSS-class keyword present in the element.
GRADE_COLOUR: dict[str, str] = {
    # Current league tiers
    "DIAMOND": "green",
    "PLATINUM": "green",
    "GOLD": "yellow",
    "SILVER": "yellow",
    "BRONZE": "red",
    # Legacy labels (backward compat — still in older DB rows)
    "EXEMPLARY": "green",
    "ESTABLISHED": "green",
    "DEVELOPING": "yellow",
    "EMERGING": "yellow",
    "NASCENT": "red",
}


def _assert_badge_colour(page: Page, grade: str, expected_colour: str) -> None:
    """
    Find a <span> whose visible text matches *grade* and assert that its CSS
    class attribute contains *expected_colour* as a substring.

    The compiled React bundle uses hashed class names such as
    ``_green_86n0l_63`` and ``_green_12hxb_58`` — both contain the colour
    keyword, so a substring check is both reliable and bundle-version agnostic.
    """
    locator = page.locator(f"span:has-text('{grade}')").first
    locator.wait_for(state="visible", timeout=5_000)
    class_attr = locator.get_attribute("class") or ""
    assert expected_colour in class_attr, (
        f"Badge for '{grade}' expected class containing '{expected_colour}', "
        f"got: '{class_attr}'"
    )


# ---------------------------------------------------------------------------
# History view
# ---------------------------------------------------------------------------


class TestHistoryViewBadges:
    """
    The History view (/history tab) fetches GET /scans and renders each scan's
    ``maturity_grade`` inside a <span> whose class encodes the colour tier.
    """

    @staticmethod
    def _go(page: Page, frontend_url: str) -> None:
        page.goto(frontend_url)
        # The History tab is labelled "History" in the navigation
        history_tab = page.locator("text=History").first
        history_tab.click()
        # Wait for the grade cells to appear
        page.wait_for_selector("td:has(span)", timeout=5_000)

    # -- Current league tiers ------------------------------------------------

    @pytest.mark.parametrize(
        "grade,colour",
        [
            ("DIAMOND", "green"),
            ("PLATINUM", "green"),
            ("GOLD", "yellow"),
            ("SILVER", "yellow"),
            ("BRONZE", "red"),
        ],
    )
    def test_current_tier_badge_colour(
        self, page: Page, frontend_url: str, grade: str, colour: str
    ) -> None:
        self._go(page, frontend_url)
        _assert_badge_colour(page, grade, colour)

    # -- Legacy backward-compat labels ---------------------------------------

    @pytest.mark.parametrize(
        "grade,colour",
        [
            ("EXEMPLARY", "green"),
            ("ESTABLISHED", "green"),
            ("DEVELOPING", "yellow"),
            ("EMERGING", "yellow"),
            ("NASCENT", "red"),
        ],
    )
    def test_legacy_badge_colour(
        self, page: Page, frontend_url: str, grade: str, colour: str
    ) -> None:
        self._go(page, frontend_url)
        _assert_badge_colour(page, grade, colour)


# ---------------------------------------------------------------------------
# Repos view
# ---------------------------------------------------------------------------


class TestReposViewBadges:
    """
    The Repos view (default/root tab) fetches GET /repos and renders each
    repo's ``latest_maturity_grade`` as a badge <span>.
    """

    @staticmethod
    def _go(page: Page, frontend_url: str) -> None:
        page.goto(frontend_url)
        # The Repos view is the default landing page; wait for table rows.
        page.wait_for_selector("td", timeout=5_000)

    # -- Current league tiers ------------------------------------------------

    @pytest.mark.parametrize(
        "grade,colour",
        [
            ("DIAMOND", "green"),
            ("PLATINUM", "green"),
            ("GOLD", "yellow"),
            ("SILVER", "yellow"),
            ("BRONZE", "red"),
        ],
    )
    def test_current_tier_badge_colour(
        self, page: Page, frontend_url: str, grade: str, colour: str
    ) -> None:
        self._go(page, frontend_url)
        _assert_badge_colour(page, grade, colour)

    # -- Legacy backward-compat labels ---------------------------------------

    @pytest.mark.parametrize(
        "grade,colour",
        [
            ("EXEMPLARY", "green"),
            ("ESTABLISHED", "green"),
            ("DEVELOPING", "yellow"),
            ("EMERGING", "yellow"),
            ("NASCENT", "red"),
        ],
    )
    def test_legacy_badge_colour(
        self, page: Page, frontend_url: str, grade: str, colour: str
    ) -> None:
        self._go(page, frontend_url)
        _assert_badge_colour(page, grade, colour)
