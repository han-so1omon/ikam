import { defineConfig, devices } from '@playwright/test';
import dotenv from 'dotenv';
import path from 'path';

dotenv.config({ path: path.resolve(__dirname, '../.env') });

export default defineConfig({
  testDir: './tests',
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: 1, // Stagehand AI calls often need serial execution for stability
  timeout: 600_000, // 10 minutes - Stagehand AI actions are slow
  reporter: 'html',
  use: {
    baseURL: process.env.IKAM_STAGEHAND_VIEWER_URL || 'http://localhost:5179',
    trace: 'on-first-retry',
    video: 'on',
    screenshot: 'on',
    actionTimeout: 60_000,
    navigationTimeout: 60_000,
    launchOptions: {
      args: [
        '--use-gl=angle',
        '--use-angle=swiftshader', // forces software rendering for WebGL in headless mode
        '--ignore-gpu-blocklist',
        '--enable-webgl',
      ]
    }
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
});
