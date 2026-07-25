// Headless repro driver for the "blank column" bug.
const puppeteer = require('puppeteer-core')

async function main() {
  const browser = await puppeteer.launch({
    executablePath: 'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
    headless: 'new',
    args: ['--window-size=1280,760'],
    defaultViewport: { width: 1265, height: 724 },
  })
  const page = await browser.newPage()
  page.on('console', (m) => console.log('[console]', m.text()))
  page.on('pageerror', (e) => console.log('[pageerror]', e.message))
  await page.goto('http://127.0.0.1:5199/', { waitUntil: 'networkidle0' })
  await new Promise((r) => setTimeout(r, 800))

  // Simulate Paste & Go with a full FILE path (deep, scrolls right)
  await page.evaluate(() => {
    const tabs = window.__tabs
    tabs.updateTab(tabs.activeTab.id, { currentPath: '/data/jenkins/release/81/common.sh' })
  })
  await new Promise((r) => setTimeout(r, 2500))

  // Maximize the preview (columns get display:none via v-show)
  await page.click('.preview-actions .toggle-btn[title="Maximize"]')
  await new Promise((r) => setTimeout(r, 300))

  // Window grows while maximized (e.g. user resizes / DPI change)
  await page.setViewport({ width: 1600, height: 724 })
  await new Promise((r) => setTimeout(r, 500))

  // Back to a shallow DIR path (few columns): preview must sit right after last column
  await page.evaluate(() => {
    const tabs = window.__tabs
    tabs.updateTab(tabs.activeTab.id, { currentPath: '/rel/81/common.sh' })
  })
  await new Promise((r) => setTimeout(r, 2500))

  // Restore (may already be restored if preview was reset)
  const restoreBtn = await page.$('.preview-actions .toggle-btn[title="Restore"]')
  if (restoreBtn) {
    await restoreBtn.click()
    await new Promise((r) => setTimeout(r, 800))
  } else {
    console.log('[driver] no Restore button — preview not maximized anymore')
  }

  const info = await page.evaluate(() => {
    const body = document.querySelector('.body')
    const cols = document.querySelector('.columns')
    const out = {
      bodyScrollLeft: body?.scrollLeft,
      bodyScrollWidth: body?.scrollWidth,
      bodyClientWidth: body?.clientWidth,
      children: [],
    }
    for (const c of cols?.children ?? []) {
      const r = c.getBoundingClientRect()
      out.children.push({
        cls: c.className,
        x: Math.round(r.x),
        w: Math.round(r.width),
        entries: c.querySelectorAll('.entry').length,
        firstText: c.querySelector('.entry-name')?.textContent?.trim(),
      })
    }
    const prev = document.querySelector('.preview-col')
    if (prev) {
      const r = prev.getBoundingClientRect()
      out.preview = { x: Math.round(r.x), w: Math.round(r.width) }
    }
    return out
  })
  console.log(JSON.stringify(info, null, 2))
  await page.screenshot({ path: 'D:\\workspace\\ShuttleSFTP\\mocktest\\repro.png' })
  await browser.close()
}

main().catch((e) => {
  console.error(e)
  process.exit(1)
})
