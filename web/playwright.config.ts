import { defineConfig } from '@playwright/test';
import { existsSync } from 'node:fs';
import { homedir } from 'node:os';
const cached = `${homedir()}/Library/Caches/ms-playwright/chromium-1243/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing`;
export default defineConfig({
  testDir: './e2e', workers: 1, timeout: 90_000,
  use: { baseURL: 'http://127.0.0.1:18081', headless: true, launchOptions: { executablePath: process.env.CHROMIUM_PATH || (existsSync(cached) ? cached : undefined) } },
  webServer: {
    command: 'MARKET_AUTO_REFRESH=0 MARKET_BIND=127.0.0.1:18081 MARKET_DATA_DIR=$(mktemp -d /private/tmp/market-e2e.XXXXXX) MARKET_WEB_DIR=web/dist target/release/stock-market',
    cwd: '..', url: 'http://127.0.0.1:18081/health', reuseExistingServer: false, timeout: 20_000,
  },
});
