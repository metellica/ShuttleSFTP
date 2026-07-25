// UI smoke test of the container/pod connect flow + cross-tab copy entry.
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

  // App boots with the connect dialog open on a fresh tab
  if (!(await page.$('.mode-switch'))) fail('mode switch not rendered')
  else ok('connect dialog with mode switch rendered')

  // Switch to Container mode: the mock lists two containers
  const modeBtns = await page.$$('.mode-btn')
  await modeBtns[1].click()
  await sleep(500)
  const items = await page.$$eval('.pick-item', (els) => els.map((e) => e.textContent.trim()))
  if (items.length !== 2 || !items[0].includes('redis')) fail('container list wrong: ' + JSON.stringify(items))
  else ok('container picker lists mock containers: ' + items.map((i) => i.split(/\s+/)[1]).join(', '))

  // Filter narrows the list
  await page.type('input[placeholder="filter by name / image / id"]', 'nginx')
  await sleep(200)
  const filtered = await page.$$eval('.pick-item', (els) => els.length)
  if (filtered !== 1) fail('filter did not narrow to 1, got ' + filtered)
  else ok('container filter works')

  // Select + connect → tab becomes a container tab with the ▣ icon
  await page.click('.pick-item')
  await page.click('.btn.primary')
  await sleep(600)
  const tabInfo = await page.evaluate(() => {
    const t = window.__tabs.activeTab
    return JSON.parse(JSON.stringify({ kind: t.kind, label: t.label, sessionId: t.sessionId, spec: t.containerSpec }))
  })
  if (tabInfo.kind !== 'container' || tabInfo.sessionId !== 'mock-container-session')
    fail('container tab not created: ' + JSON.stringify(tabInfo))
  else ok('container connect creates tab: ' + tabInfo.label)
  if (!tabInfo.spec || tabInfo.spec.preferRootfs !== true) fail('containerSpec missing preferRootfs')
  else ok('containerSpec recorded for bookmarking (preferRootfs=true)')

  const kindIcon = await page.$eval('.tab.active .tab-kind', (e) => e.textContent.trim())
  if (kindIcon !== '▣') fail('tab kind icon wrong: ' + kindIcon)
  else ok('tab bar shows container icon ▣')

  // K8s pod flow: new tab → pod mode → namespace/pod pickers
  await page.click('.tab-add')
  await sleep(400)
  const modeBtns2 = await page.$$('.mode-btn')
  await modeBtns2[2].click()
  await sleep(500)
  const nsOptions = await page.$$eval('.field select', (sels) =>
    sels.map((s) => Array.from(s.options).map((o) => o.value))
  )
  const hasNs = nsOptions.some((opts) => opts.includes('default') && opts.includes('kube-system'))
  if (!hasNs) fail('namespaces not loaded: ' + JSON.stringify(nsOptions))
  else ok('pod mode loads contexts + namespaces')
  await sleep(400)
  const podItems = await page.$$eval('.pick-item', (els) => els.map((e) => e.textContent.trim()))
  if (!podItems.some((p) => p.includes('nginx-7d4'))) fail('pods not listed: ' + JSON.stringify(podItems))
  else ok('pod picker lists mock pods')

  await page.click('.pick-item')
  await sleep(200)
  // Multi-container pod exposes a container select
  const podConnected = await page.$$('.field select')
  await page.click('.btn.primary')
  await sleep(500)
  const podTab = await page.evaluate(() => {
    const t = window.__tabs.activeTab
    return JSON.parse(JSON.stringify({ kind: t.kind, label: t.label, spec: t.podSpec }))
  })
  if (podTab.kind !== 'pod' || !podTab.spec || podTab.spec.namespace !== 'default')
    fail('pod tab not created: ' + JSON.stringify(podTab))
  else ok('pod connect creates tab: ' + podTab.label)

  // Cross-tab copy entry: context menu on a file should offer "Copy to"
  // (needs the file panel of the container tab + at least one other tab)
  await page.evaluate(() => {
    const tabs = window.__tabs
    const other = tabs.tabs.find((t) => t.kind === 'container')
    tabs.setActiveTab(other.id)
  })
  await sleep(800)
  const rows = await page.$$('.entry')
  if (rows.length === 0) fail('no file entries rendered in container tab')
  else {
    await rows[0].click({ button: 'right' })
    await sleep(300)
    const ctxItems = await page.$$eval('.ctx-menu .ctx-item', (els) =>
      els.map((e) => e.textContent.trim())
    )
    const copyTo = ctxItems.find((t) => t.startsWith('📤 Copy to'))
    if (!copyTo) fail('Copy to entry missing: ' + JSON.stringify(ctxItems))
    else ok('context menu offers Copy to ▸')
    // Expand the submenu: it should list the pod tab as a target
    const btns = await page.$$('.ctx-menu .ctx-item')
    for (const b of btns) {
      const t = await b.evaluate((e) => e.textContent.trim())
      if (t.startsWith('📤 Copy to')) {
        await b.click()
        break
      }
    }
    await sleep(200)
    const subItems = await page.$$eval('.ctx-menu .ctx-sub', (els) =>
      els.map((e) => e.textContent.trim())
    )
    if (subItems.length === 0) fail('Copy to submenu empty')
    else ok('Copy to targets: ' + subItems.join(' | '))
  }

  // Tab context menu on an SSH tab offers "Browse containers"
  await page.evaluate(() => {
    const tabs = window.__tabs
    const t = tabs.addTab()
    tabs.updateTab(t.id, { sessionId: 'mock-ssh', label: 'root@host1', status: 'connected', kind: 'ssh' })
  })
  await sleep(300)
  const sshTab = await page.$$('.tab')
  await sshTab[sshTab.length - 1].click({ button: 'right' })
  await sleep(200)
  const tabCtx = await page.$$eval('.tab-bar .ctx-item', (els) => els.map((e) => e.textContent.trim()))
  if (!tabCtx.some((t) => t.includes('Browse containers'))) fail('Browse containers entry missing: ' + JSON.stringify(tabCtx))
  else ok('SSH tab context menu offers Browse containers…')

  await page.screenshot({ path: 'mocktest/container-flow.png' })
  await browser.close()
  console.log(process.exitCode === 1 ? '\nSMOKE TEST FAILED' : '\nSMOKE TEST PASSED')
}

main().catch((e) => {
  console.error(e)
  process.exit(1)
})
