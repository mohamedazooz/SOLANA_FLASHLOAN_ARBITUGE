import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const source = (path) => readFileSync(new URL(path, import.meta.url), 'utf8');

test('login requires explicit operator password submission', () => {
  const login = source('./src/components/Login.tsx');
  assert.match(login, /api\.login\(password\)/);
  assert.doesNotMatch(login, /defaultPassword|api\.login\(['"]1234['"]\)/);
  assert.doesNotMatch(login, /useEffect\s*\(/);
});

test('dashboard and logs obtain short-lived WebSocket tickets', () => {
  for (const path of ['./src/App.tsx', './src/components/Logs.tsx']) {
    const component = source(path);
    assert.match(component, /api\.wsUrl\(\)/, path);
    assert.doesNotMatch(component, /\?token=|getToken\(\).*WebSocket/, path);
  }
  const api = source('./src/api.ts');
  assert.match(api, /api\.wsTicket\(\)/);
  assert.match(api, /\/api\/ws\?ticket=/);
});

test('dashboard emergency controls use the canonical control API', () => {
  const app = source('./src/App.tsx');
  assert.match(app, /api\.control\('emergency-stop'\)/);
  assert.match(app, /api\.control\('clear-emergency-stop'\)/);
  assert.match(app, /api\.control\('start'\)/);
  assert.doesNotMatch(app, /updateConfig\(\{[^}]*emergency_stopped/s);

  const riskControl = source('./src/components/RiskControl.tsx');
  assert.match(riskControl, /api\.control\("emergency-stop"\)/);
  assert.match(riskControl, /api\.control\("clear-emergency-stop"\)/);
});
