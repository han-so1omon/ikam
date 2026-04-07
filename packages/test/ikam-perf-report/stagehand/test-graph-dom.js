const { chromium } = require('playwright');

(async () => {
  const browser = await chromium.launch();
  const context = await browser.newContext();
  const page = await context.newPage();
  
  await page.goto('http://localhost:5179');
  await page.waitForTimeout(3000);
  
  // Try to find the run and click graph
  await page.getByRole('button', { name: 'Graph', exact: true }).click();
  await page.waitForTimeout(3000);
  
  const html = await page.content();
  console.log('Is loading graph map in HTML?', html.includes('Loading graph map...'));
  console.log('Is graph-map in HTML?', html.includes('graph-map'));
  
  await browser.close();
})();
