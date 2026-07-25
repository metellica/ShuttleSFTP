// UI smoke test: local session + virtual /@containers and /@pods dirs.
const puppeteer = require('puppeteer-core')

const fail = (msg) => {
  console.error('[FAIL]', msg)
  process.exitCode = 1
}
const ok = (msg) => console.log('[OK]', msg)
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

async function main() {
  const browser = await puppeteer.launch({
    executablePath: 'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
    headless: 'new',
    args: ['--window-size=1280,760'],
    defaultViewport: { width: 1265, height: 724 },
  })
  const page = await browser.newPage()
  page.on('pageerror', (e) => fail('pageerror: ' + e.message))
  await page.goto('http://localhost:5199/mocktest/app.html', { waitUntil: 'networkidle0' })
  await sleep(600)

  // Connect dialog with SSH | This Machine switch
  const modeLabels = await page.$$eval('.mode-btn', (els) => els.map((e) => e.textContent.trim()))
  if (modeLabels.length !== 2 || !modeLabels[1].includes('This Machine'))
    fail('mode switch wrong: ' + JSON.stringify(modeLabels))
  else ok('connect dialog offers SSH | This Machine')

  // Open a local session
  const btns = await page.$$('.mode-btn')
  await btns[1].click()
  await sleep(200)
  await page.click('.btn.primary')
  await sleep(700)
  const tab = await page.evaluate(() =>
    JSON.parse(
      JSON.stringify({
        kind: window.__tabs.activeTab.kind,
        label: window.__tabs.activeTab.label,
        sessionId: window.__tabs.activeTab.sessionId,
      })
    )
  )
  if (tab.kind !== 'local' || tab.sessionId !== 'mock-local-session')
    fail('local tab not created: ' + JSON.stringify(tab))
  else ok('local session tab created: ' + tab.label)

  // Root listing shows virtual dirs with icons
  const entries = await page.$$eval('.entry', (els) =>
    els.map((e) => e.textContent.trim().replace(/\s+/g, ' '))
  )
  const vc = entries.find((t) => t.includes('@containers'))
  const vp = entries.find((t) => t.includes('@pods'))
  if (!vc || !vp) fail('virtual dirs missing from root: ' + JSON.stringify(entries.slice(0, 6)))
  else ok('root lists @containers and @pods')
  if (!vc.includes('▣') || !vp.includes('⎈')) fail('virtual dir icons wrong: ' + vc + ' | ' + vp)
  else ok('virtual dirs use ▣ / ⎈ icons')

  // Navigate: @containers -> redis -> files
  await page.evaluate(() => {
    const tabs = window.__tabs
    tabs.updateTab(tabs.activeTab.id, { currentPath: '/@containers/redis' })
  })
  await sleep(900)
  const inRedis = await page.$$eval('.entry', (els) => els.map((e) => e.textContent.trim()))
  if (!inRedis.some((t) => t.includes('redis.conf')))
    fail('container files not listed: ' + JSON.stringify(inRedis.slice(-4)))
  else ok('navigating /@containers/redis lists its files')

  // Breadcrumb reflects the virtual path
  const crumbs = await page.$$eval('.crumb', (els) => els.map((e) => e.textContent.trim()))
  if (!crumbs.includes('@containers') || !crumbs.includes('redis'))
    fail('breadcrumbs wrong: ' + JSON.stringify(crumbs))
  else ok('breadcrumb shows / › @containers › redis')

  // Pod path navigation
  await page.evaluate(() => {
    const tabs = window.__tabs
    tabs.updateTab(tabs.activeTab.id, { currentPath: '/@pods/default/nginx-7d4/nginx' })
  })
  await sleep(900)
  const inPod = await page.$$eval('.entry', (els) => els.map((e) => e.textContent.trim()))
  if (!inPod.some((t) => t.includes('nginx.conf')))
    fail('pod files not listed: ' + JSON.stringify(inPod.slice(-4)))
  else ok('navigating /@pods/default/nginx-7d4/nginx lists its files')

  // Context menu on a file offers Copy; Paste is disabled until something is copied
  const fileEntry = (await page.$$('.entry'))[0]
  await fileEntry.click({ button: 'right' })
  await sleep(300)
  let ctxItems = await page.$$eval('.ctx-menu .ctx-item', (els) =>
    els.map((e) => ({ text: e.textContent.trim(), disabled: e.disabled }))
  )
  const copyItem = ctxItems.find((t) => t.text.startsWith('📋 Copy'))
  const pasteItem = ctxItems.find((t) => t.text.startsWith('📥 Paste'))
  if (!copyItem) fail('Copy missing: ' + JSON.stringify(ctxItems))
  else ok('context menu offers Copy')
  if (!pasteItem || !pasteItem.disabled) fail('Paste should be disabled before Copy')
  else ok('Paste disabled while clipboard is empty')

  // Copy the file, then Paste becomes enabled
  for (const b of await page.$$('.ctx-menu .ctx-item')) {
    if ((await b.evaluate((e) => e.textContent.trim())).startsWith('📋 Copy')) {
      await b.click()
      break
    }
  }
  await sleep(200)
  await (await page.$$('.entry'))[0].click({ button: 'right' })
  await sleep(300)
  ctxItems = await page.$$eval('.ctx-menu .ctx-item', (els) =>
    els.map((e) => ({ text: e.textContent.trim(), disabled: e.disabled }))
  )
  const paste2 = ctxItems.find((t) => t.text.startsWith('📥 Paste'))
  if (!paste2 || paste2.disabled) fail('Paste not enabled after Copy: ' + JSON.stringify(ctxItems))
  else ok('Paste enabled after Copy: ' + paste2.text)

  // Blank-area context menu offers Paste / New Folder / Refresh
  await page.keyboard.press('Escape')
  await page.click('body')
  await sleep(200)
  // /@containers/redis has few entries, its column is mostly blank
  await page.evaluate(() => {
    const tabs = window.__tabs
    tabs.updateTab(tabs.activeTab.id, { currentPath: '/@containers/redis' })
  })
  await sleep(900)
  const cols = await page.$$('.column')
  const lastCol = cols[cols.length - 1]
  const box = await lastCol.boundingBox()
  await page.mouse.click(box.x + box.width / 2, box.y + box.height - 20, { button: 'right' })
  await sleep(300)
  const blankItems = await page.$$eval('.ctx-menu .ctx-item', (els) =>
    els.map((e) => e.textContent.trim())
  )
  if (!blankItems.some((t) => t.startsWith('📥 Paste')) || !blankItems.some((t) => t.includes('New Folder')))
    fail('blank-area menu wrong: ' + JSON.stringify(blankItems))
  else ok('blank-area right-click offers Paste / New Folder / Refresh: ' + blankItems[0])

  await page.screenshot({ path: 'mocktest/virtual-dirs.png' })
  await browser.close()
  console.log(process.exitCode === 1 ? '\nSMOKE TEST FAILED' : '\nSMOKE TEST PASSED')
}

main().catch((e) => {
  console.error(e)
  process.exit(1)
})
