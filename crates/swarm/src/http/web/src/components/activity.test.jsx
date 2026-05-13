import { afterEach, describe, expect, test, vi } from 'vitest';
import React from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import '@testing-library/jest-dom/vitest';

import { ApiProvider } from '../hooks/useApi.jsx';
import { ActivityPage } from './activity.jsx';
import { listToolCalls, streamToolCalls } from '../api/audit.js';

vi.mock('../api/audit.js', () => ({
  listToolCalls: vi.fn(),
  exportToolCallsNdjson: vi.fn(),
  streamToolCalls: vi.fn(() => () => {}),
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.useRealTimers();
});

function renderActivity(client = {}) {
  return render(
    <ApiProvider client={client} auth={{ mode: 'none' }}>
      <ActivityPage instanceId="inst-a" embedded/>
    </ApiProvider>,
  );
}

const rows = [{
  id: 1,
  llm_audit_id: 7,
  instance_id: 'inst-a',
  tool_use_id: 'call-1',
  tool_name: 'bash',
  mcp_server: null,
  input: { cmd: 'pwd' },
  result: { stdout: '/workspace' },
  is_error: false,
  called_at: 1760000000,
  resulted_at: 1760000002,
  mcp_audit_id: null,
  mcp_status: null,
  mcp_duration_ms: null,
}];

describe('ActivityPage', () => {
  test('renders the empty state without filters', async () => {
    listToolCalls.mockResolvedValue({ items: [], next_cursor: null });
    renderActivity();

    expect(await screen.findByText(/no tool calls yet/i)).toBeInTheDocument();
    expect(screen.queryByLabelText('tool filter')).toBeNull();
    expect(streamToolCalls).toHaveBeenCalled();
  });

  test('renders rows, opens the drawer, and applies filters', async () => {
    listToolCalls.mockResolvedValue({ items: rows, next_cursor: 1 });
    renderActivity();

    expect((await screen.findAllByText('bash')).length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole('listitem'));
    expect(screen.getByRole('dialog', { name: /tool call detail/i })).toBeInTheDocument();
    expect(screen.getByText('call-1')).toBeInTheDocument();
    expect(screen.getByText(/workspace/)).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('status filter'), { target: { value: 'ok' } });
    await waitFor(() => expect(listToolCalls).toHaveBeenLastCalledWith(
      expect.anything(),
      'inst-a',
      expect.objectContaining({ status: 'ok' }),
    ));
  });

  test('updates an open drawer when a result attaches to an existing row', async () => {
    const pending = {
      ...rows[0],
      result: null,
      is_error: null,
      resulted_at: null,
    };
    const completed = {
      ...pending,
      result: { stdout: 'audit-smoke' },
      is_error: false,
      resulted_at: pending.called_at + 3,
    };
    let pushToolCall;
    streamToolCalls.mockImplementationOnce((client, instanceId, filters, onEvent) => {
      pushToolCall = onEvent;
      return () => {};
    });
    listToolCalls.mockResolvedValue({ items: [pending], next_cursor: pending.id });
    renderActivity();

    fireEvent.click(await screen.findByRole('listitem'));
    expect(screen.queryByText(/audit-smoke/)).toBeNull();

    act(() => pushToolCall(completed));

    expect(await screen.findByText(/audit-smoke/)).toBeInTheDocument();
  });
});
