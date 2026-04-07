import { test, expect } from '@playwright/test';
import { createStagehand } from '../src/lib/stagehand';
import { runIngestionJourney } from '../src/journeys/ingestion';

test.describe('IKAM Ingestion Performance Report', () => {
  let stagehand: any;

  test.beforeAll(async () => {
    stagehand = await createStagehand();
  });

  test.afterAll(async () => {
    await stagehand.close();
  });

  test('User Journey: s-local-retail-v01 Ingestion Step-Through', async ({ page }) => {
    test.setTimeout(600_000); // 10 minutes
    // Stagehand uses its own internal page, but we can sync or just use Stagehand's
    const result = await runIngestionJourney(page, stagehand, 's-local-retail-v01');
    
    console.log('Journey Result:', JSON.stringify(result, null, 2));

    expect(result.graphAnalysis.looksNice).toBe(true);
    expect(result.graphAnalysis.consistencyWithStepThrough).toBe(true);
    expect(result.graphAnalysis.issues.length).toBeLessThan(3);
  });
});
