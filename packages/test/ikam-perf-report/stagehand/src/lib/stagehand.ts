import { Stagehand } from '@browserbasehq/stagehand';

export async function createStagehand() {
  const stagehand = new Stagehand({
    env: 'LOCAL',
    apiKey: process.env.OPENAI_API_KEY,
    modelName: (process.env.OPENAI_MODEL || 'gpt-4o-mini') as any,
    headless: true,
  });
  await stagehand.init();
  return stagehand;
}
