import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { pathAfterExecutableEdit, primaryExecutable } from '../src/libraryState.ts';

const component = readFileSync(new URL('../src/components/AppsManager.tsx', import.meta.url), 'utf8');
const handler = component.match(/value=\{newExe\}\s+onChange=\{([\s\S]+?)\}\s+className=/)?.[1];

function manualForm(pickedPath) {
  assert.ok(handler, 'the executable input must have a change handler');
  const state = { exe: 'game.exe', path: pickedPath, hdrMatched: true };
  const helperCalls = [];
  const onChange = runInNewContext(`(${handler})`, {
    setNewExe: (value) => { state.exe = value; },
    setNewPath: (update) => {
      assert.equal(typeof update, 'function', 'path invalidation must use the current picked path');
      state.path = update(state.path);
    },
    setIsHdrMatched: (value) => { state.hdrMatched = value; },
    pathAfterExecutableEdit: (path, value) => {
      helperCalls.push([path, value]);
      return pathAfterExecutableEdit(path, value);
    },
  });
  return { state, helperCalls, edit: (value) => onChange({ target: { value } }) };
}

test('manual executable input wires the shared path helper into the actual change handler', () => {
  assert.match(component, /import\s*\{[^}]*\bpathAfterExecutableEdit\b[^}]*\}\s*from\s*['"]\.\.\/libraryState['"]/);
  assert.match(handler, /setNewPath\(\(path\)\s*=>\s*pathAfterExecutableEdit\(path,\s*e\.target\.value\)\)/);
  assert.match(component, /const cleanExe = primaryExecutable\(newExe\)/);
  assert.match(component, /const newApp: HdrApp = \{[^}]*exe_name: cleanExe,[^}]*path: newPath \|\| undefined,/);
});

test('editing the primary executable clears the picked path through the rendered handler', () => {
  const pickedPath = 'D:\\Games\\game.exe';
  const form = manualForm(pickedPath);
  form.edit('renderer');
  assert.deepEqual(form.helperCalls, [[pickedPath, 'renderer']]);
  assert.equal(primaryExecutable(form.state.exe), 'renderer.exe');
  assert.equal(form.state.path, '');
  assert.equal(form.state.path || undefined, undefined);
  assert.equal(form.state.hdrMatched, false);
});

test('equivalent executable edits preserve the picked path without restoring HDR matching', () => {
  for (const [path, edit] of [
    ['D:\\Games\\game.exe', '  GAME.EXE  '],
    ['D:/Games/game.exe', 'game'],
    ['\\\\?\\D:\\Games\\.\\game.exe', 'GAME'],
  ]) {
    const form = manualForm(path);
    form.edit(edit);
    assert.equal(form.state.path, path);
    assert.equal(primaryExecutable(form.state.exe), 'game.exe');
    assert.equal(form.state.hdrMatched, false);
    assert.deepEqual(form.helperCalls, [[path, edit]]);
  }
});

test('clearing or changing the primary cannot resurrect an old picked binding on a later edit', () => {
  for (const firstEdit of ['', 'renderer.exe']) {
    const form = manualForm('D:\\Games\\game.exe');
    form.edit(firstEdit);
    assert.equal(form.state.path, '');
    form.edit('game.exe');
    assert.equal(form.state.path, '');
    assert.deepEqual(form.helperCalls, [['D:\\Games\\game.exe', firstEdit], ['', 'game.exe']]);
  }
});
