// Custom Playwright test fixtures for the end-to-end harness.
//
//
// Provides:
//   - mockTauri: installs the @tauri-apps/api/mocks bridge before navigation
//   - axe: a configured AxeBuilder bound to the current page
//   - setLocale: navigates with a ?locale= query param so locale-switching
//     specs survive both the no-locale-UI baseline and the
//     movable-do solfege drop-in.

import { test as base, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { installTauriMock, type TauriMockResponses } from "./helpers/tauri-mock";

export interface MockTauri {
  install: (responses?: TauriMockResponses) => Promise<void>;
}

export interface NeuralPitchFixtures {
  mockTauri: MockTauri;
  axe: AxeBuilder;
  setLocale: (locale: string) => Promise<void>;
}

/**
 * `test` extends the base Playwright test with NeuralPitch-specific fixtures.
 * Specs import this `test` rather than the base one so the mock bridge is
 * always available without per-spec boilerplate.
 */
export const test = base.extend<NeuralPitchFixtures>({
  mockTauri: async ({ page }, use) => {
    const helper: MockTauri = {
      install: async (responses?: TauriMockResponses) => {
        await installTauriMock(page, responses ?? {});
      },
    };
    await use(helper);
  },
  axe: async ({ page }, use) => {
    const builder = new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]);
    await use(builder);
  },
  setLocale: async ({ page }, use) => {
    const navigate = async (locale: string): Promise<void> => {
      await navigateWithLocale(page, locale);
    };
    await use(navigate);
  },
});

async function navigateWithLocale(page: Page, locale: string): Promise<void> {
  await page.goto(`/?locale=${encodeURIComponent(locale)}`);
}

export { expect } from "@playwright/test";
