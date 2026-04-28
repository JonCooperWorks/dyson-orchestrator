/* Tests for the SPA's tiny external store.
 *
 * Locks down the contract React's useSyncExternalStore expects:
 *   - getSnapshot returns the same reference until a real update lands
 *   - subscribers fire on change and only on change
 *   - returning the same reference from a reducer skips notification
 *   - the published snapshot is deep-frozen (catches in-place mutations)
 */
import { describe, expect, test, vi } from 'vitest';

import { createStore, deepFreeze } from './createStore.js';

describe('deepFreeze', () => {
  test('freezes nested objects and arrays', () => {
    const o = deepFreeze({ a: 1, b: { c: [1, 2, { d: 3 }] } });
    expect(Object.isFrozen(o)).toBe(true);
    expect(Object.isFrozen(o.b)).toBe(true);
    expect(Object.isFrozen(o.b.c)).toBe(true);
    expect(Object.isFrozen(o.b.c[2])).toBe(true);
  });

  test('returns primitives unchanged', () => {
    expect(deepFreeze(42)).toBe(42);
    expect(deepFreeze(null)).toBeNull();
    expect(deepFreeze('s')).toBe('s');
  });

  test('idempotent on already-frozen values', () => {
    const o = Object.freeze({ x: 1 });
    expect(deepFreeze(o)).toBe(o);
  });
});

describe('createStore', () => {
  test('getSnapshot returns the initial value, deep-frozen', () => {
    const s = createStore({ instances: [] });
    expect(s.getSnapshot()).toEqual({ instances: [] });
    expect(Object.isFrozen(s.getSnapshot())).toBe(true);
  });

  test('dispatch with a same-ref reducer does not notify subscribers', () => {
    const s = createStore({ count: 0 });
    const fn = vi.fn();
    s.subscribe(fn);
    s.dispatch((prev) => prev);
    expect(fn).not.toHaveBeenCalled();
  });

  test('dispatch with a new value notifies subscribers and updates the snapshot', () => {
    const s = createStore({ count: 0 });
    const fn = vi.fn();
    s.subscribe(fn);
    s.dispatch((prev) => ({ ...prev, count: prev.count + 1 }));
    expect(fn).toHaveBeenCalledTimes(1);
    expect(s.getSnapshot()).toEqual({ count: 1 });
  });

  test('subscribe returns an unsubscribe function', () => {
    const s = createStore({});
    const fn = vi.fn();
    const unsub = s.subscribe(fn);
    s.dispatch(() => ({ a: 1 }));
    expect(fn).toHaveBeenCalledTimes(1);
    unsub();
    s.dispatch(() => ({ a: 2 }));
    expect(fn).toHaveBeenCalledTimes(1); // still 1 — listener was removed
  });

  test('snapshot reference is stable across no-op dispatches (useSyncExternalStore contract)', () => {
    const s = createStore({ a: 1 });
    const before = s.getSnapshot();
    s.dispatch((prev) => prev);
    expect(s.getSnapshot()).toBe(before);
  });

  test('regression: in-place mutation of the published snapshot throws (deep-frozen)', () => {
    const s = createStore({ list: [1, 2, 3] });
    const snap = s.getSnapshot();
    expect(() => {
      'use strict';
      snap.list.push(4);
    }).toThrow();
  });
});
