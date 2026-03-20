"""
Phase 5 — Playwright e2e tests: force-directed layout toggle.

These tests are RED until Phase 5 is implemented.  They specify the exact
UI contract (data-testid attributes, default state, toggle behaviour) so the
implementation can be written to make them green without guessing intent.

All backend API calls are intercepted by the ``mock_api`` autouse fixture in
``conftest.py`` — no running Rust server required.
"""

import pytest
from playwright.sync_api import Page, expect

_TIMEOUT = 8_000
_GRAPH_TIMEOUT = 12_000


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def go_to_graph(page: Page, frontend_url: str) -> None:
    """Navigate to the app and switch to the Graph tab."""
    page.goto(frontend_url)
    page.locator("nav button", has_text="Graph").click()
    expect(page.locator('[data-testid="decision-graph-view"]')).to_be_visible(
        timeout=_GRAPH_TIMEOUT
    )


# ---------------------------------------------------------------------------
# Suite: Layout toggle presence
# ---------------------------------------------------------------------------


class TestLayoutTogglePresence:
    def test_layout_toggle_is_visible_in_layout_panel(
        self, page: Page, frontend_url: str
    ) -> None:
        """The layout panel must contain a layout-mode toggle control."""
        go_to_graph(page, frontend_url)
        expect(
            page.locator('[data-testid="layout-panel"] [data-testid="layout-toggle"]')
        ).to_be_visible(timeout=_TIMEOUT)

    def test_radial_option_is_present(self, page: Page, frontend_url: str) -> None:
        """The toggle must include a Radial option."""
        go_to_graph(page, frontend_url)
        expect(page.locator('[data-testid="layout-mode-radial"]')).to_be_visible(
            timeout=_TIMEOUT
        )

    def test_force_option_is_present(self, page: Page, frontend_url: str) -> None:
        """The toggle must include a Force option."""
        go_to_graph(page, frontend_url)
        expect(page.locator('[data-testid="layout-mode-force"]')).to_be_visible(
            timeout=_TIMEOUT
        )


# ---------------------------------------------------------------------------
# Suite: Default state
# ---------------------------------------------------------------------------


class TestLayoutToggleDefault:
    def test_radial_is_selected_by_default(self, page: Page, frontend_url: str) -> None:
        """On first load the Radial layout radio button must be checked."""
        go_to_graph(page, frontend_url)
        radial = page.locator('[data-testid="layout-mode-radial"]')
        expect(radial).to_be_checked(timeout=_TIMEOUT)

    def test_force_is_not_selected_by_default(
        self, page: Page, frontend_url: str
    ) -> None:
        """On first load the Force layout radio button must NOT be checked."""
        go_to_graph(page, frontend_url)
        force = page.locator('[data-testid="layout-mode-force"]')
        expect(force).not_to_be_checked(timeout=_TIMEOUT)

    def test_canvas_is_visible_in_radial_mode(
        self, page: Page, frontend_url: str
    ) -> None:
        """The Three.js canvas must be rendered in the default Radial mode."""
        go_to_graph(page, frontend_url)
        expect(
            page.locator('[data-testid="decision-graph-view"] canvas')
        ).to_be_visible(timeout=_GRAPH_TIMEOUT)


# ---------------------------------------------------------------------------
# Suite: Switching to Force layout
# ---------------------------------------------------------------------------


class TestLayoutToggleSwitch:
    def test_selecting_force_checks_force_radio(
        self, page: Page, frontend_url: str
    ) -> None:
        """Clicking the Force radio must mark it as checked."""
        go_to_graph(page, frontend_url)
        page.locator('[data-testid="layout-mode-force"]').click()
        expect(page.locator('[data-testid="layout-mode-force"]')).to_be_checked(
            timeout=_TIMEOUT
        )

    def test_selecting_force_unchecks_radial(
        self, page: Page, frontend_url: str
    ) -> None:
        """Switching to Force must deselect Radial."""
        go_to_graph(page, frontend_url)
        page.locator('[data-testid="layout-mode-force"]').click()
        expect(page.locator('[data-testid="layout-mode-radial"]')).not_to_be_checked(
            timeout=_TIMEOUT
        )

    def test_canvas_remains_visible_after_switching_to_force(
        self, page: Page, frontend_url: str
    ) -> None:
        """The Three.js canvas must still be rendered after switching to Force."""
        go_to_graph(page, frontend_url)
        page.locator('[data-testid="layout-mode-force"]').click()
        expect(
            page.locator('[data-testid="decision-graph-view"] canvas')
        ).to_be_visible(timeout=_GRAPH_TIMEOUT)

    def test_switching_back_to_radial_rechecks_radial(
        self, page: Page, frontend_url: str
    ) -> None:
        """Switching Force → Radial must restore the Radial checked state."""
        go_to_graph(page, frontend_url)
        page.locator('[data-testid="layout-mode-force"]').click()
        page.locator('[data-testid="layout-mode-radial"]').click()
        expect(page.locator('[data-testid="layout-mode-radial"]')).to_be_checked(
            timeout=_TIMEOUT
        )

    def test_node_list_still_populated_after_switching_to_force(
        self, page: Page, frontend_url: str
    ) -> None:
        """Node list items must still be rendered after switching to Force layout."""
        go_to_graph(page, frontend_url)
        # Confirm nodes loaded first
        expect(page.locator('[data-testid="node-list-item"]').first).to_be_visible(
            timeout=_GRAPH_TIMEOUT
        )
        # Switch layout
        page.locator('[data-testid="layout-mode-force"]').click()
        expect(page.locator('[data-testid="node-list-item"]').first).to_be_visible(
            timeout=_TIMEOUT
        )
