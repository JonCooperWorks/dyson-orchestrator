/* Tests for the cube-profile dropdown label formatter.
 *
 * Lives next to its module so the operator-visible string stays
 * stable — the SPA is the only place an operator sees these
 * profiles, and a label change shouldn't sneak in via a refactor.
 */
import { describe, expect, test } from 'vitest';

import { profileLabel } from './instances.jsx';

describe('profileLabel', () => {
  test('renders the operator-facing tuple for a whole-cpu, whole-GB profile', () => {
    // cubemastercli memory unit is MB (decimal), so 2000 MB = 2 GB.
    expect(profileLabel({
      name: 'default',
      template_id: 'tpl-default',
      disk_gb: 5,
      cpu_millicores: 2000,
      memory_mb: 2000,
    })).toBe('default — 5 GB disk · 2 vCPU · 2 GB RAM');
  });

  test('formats fractional vcpu with one decimal', () => {
    // 1500 millicores = 1.5 vCPU — shouldn't read as "1 vCPU" or "2 vCPU".
    expect(profileLabel({
      name: 'small',
      template_id: 'tpl-small',
      disk_gb: 1,
      cpu_millicores: 1500,
      memory_mb: 1000,
    })).toBe('small — 1 GB disk · 1.5 vCPU · 1 GB RAM');
  });

  test('falls back to MB when memory is not a whole multiple of 1000', () => {
    // 1500 MB ≈ 1.5 GB — would round if we naively used integer division;
    // raw MB keeps the label honest.
    expect(profileLabel({
      name: 'odd',
      template_id: 'tpl-odd',
      disk_gb: 10,
      cpu_millicores: 4000,
      memory_mb: 1500,
    })).toBe('odd — 10 GB disk · 4 vCPU · 1500 MB RAM');
  });

  test('large profiles render correctly', () => {
    expect(profileLabel({
      name: 'xl',
      template_id: 'tpl-xl',
      disk_gb: 200,
      cpu_millicores: 8000,
      memory_mb: 16000,
    })).toBe('xl — 200 GB disk · 8 vCPU · 16 GB RAM');
  });

  test('returns empty string on a falsy profile', () => {
    // The dropdown maps over an array — defending against an
    // undefined entry keeps a malformed /auth/config response from
    // crashing the whole page.
    expect(profileLabel(null)).toBe('');
    expect(profileLabel(undefined)).toBe('');
  });
});
