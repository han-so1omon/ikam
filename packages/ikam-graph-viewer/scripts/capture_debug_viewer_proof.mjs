import fs from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';

const outputDir = path.resolve('../..', 'artifacts/proof');
await fs.mkdir(outputDir, { recursive: true });

const browser = await chromium.launch({ headless: true });
const context = await browser.newContext({
  viewport: { width: 1600, height: 1100 },
  recordVideo: { dir: outputDir, size: { width: 1600, height: 1100 } },
});
const page = await context.newPage();

await page.goto('http://127.0.0.1:5173', { waitUntil: 'networkidle' });
await page.getByLabel('s-local-retail-v01').click();
await page.getByRole('button', { name: 'Run Cases' }).click();
await page.getByTestId('debug-step-list').waitFor({ timeout: 30000 });
await page.getByRole('button', { name: 'Next Step' }).click();
await page.getByTestId('step-detail-split').waitFor({ timeout: 30000 });
await page.screenshot({ path: path.join(outputDir, 'debug-content-viewer.png'), fullPage: true });

await page.close();
await context.close();
await browser.close();
