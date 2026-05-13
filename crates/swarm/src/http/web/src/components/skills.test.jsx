import { afterEach, describe, expect, test, vi } from 'vitest';
import React from 'react';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import '@testing-library/jest-dom/vitest';

import { ApiProvider } from '../hooks/useApi.jsx';
import { MarketplaceSkillDetailPage, SkillCatalogRow, SkillInventoryList } from './skills.jsx';

afterEach(() => {
  cleanup();
  window.location.hash = '';
  vi.restoreAllMocks();
});

describe('SkillCatalogRow', () => {
  test('links agent-authored marketplace skills back to the author instance', () => {
    render(
      <SkillCatalogRow
        skill={{
          marketplace_id: 'agent-axelrod',
          marketplace_name: 'Axelrod skills',
          name: 'debug-logs',
          version: '0.1.0',
          description: 'Read logs before guessing.',
          content_type: 'workspace',
          author: {
            name: 'Axelrod',
            instance_id: 'axelrod',
            href: '#/i/axelrod/skills',
          },
        }}
      />,
    );

    expect(screen.getByText('debug-logs')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /debug-logs/ }))
      .toHaveAttribute('href', '#/skills/agent-axelrod/debug-logs');
    expect(screen.getByText('learned by Axelrod')).toBeInTheDocument();
  });

  test('catalog rows constrain long descriptions inside the panel', () => {
    render(
      <SkillCatalogRow
        skill={{
          marketplace_id: 'agent-axelrod',
          marketplace_name: 'Axelrod skills',
          name: 'massive-financial-analysis',
          version: '0.1.0',
          description: 'Systematic stock analysis using the Massive MCP server: pull overview, price history, consensus ratings, analyst actions, and news; query with SQL; compile markdown investment memos. Includes debugging patterns for transport failures.'.repeat(4),
          content_type: 'workspace',
          author: {
            name: 'Axelrod',
            instance_id: 'axelrod',
            href: '#/i/axelrod/skills',
          },
        }}
      />,
    );

    expect(screen.getByRole('link', { name: /massive-financial-analysis/ }))
      .toHaveClass('skill-catalog-row');
    expect(screen.getByText(/Systematic stock analysis/))
      .toHaveClass('skill-row-description');
  });

  test('detail page renders metadata, markdown, and installs to a live instance', async () => {
    const skill = {
      marketplace_id: 'team-skills',
      marketplace_name: 'Team Skills',
      name: 'code-review',
      version: '1.0.0',
      description: 'Review code changes.',
      tags: ['review', 'code'],
      license: 'MIT',
      min_dyson_version: '0.1.0',
      sha256: 'declared',
      content_type: 'inline',
      author: null,
    };
    const client = {
      getMarketplaceSkill: vi.fn(async () => ({
        skill,
        preview: '# Code Review',
        computed_sha256: 'computed',
      })),
      getMarketplaceSkillContent: vi.fn(async () => ({
        marketplace_id: 'team-skills',
        marketplace_name: 'Team Skills',
        name: 'code-review',
        version: '1.0.0',
        description: 'Review code changes.',
        declared_sha256: 'declared',
        computed_sha256: 'computed',
        skill_md: '# Code Review\n\nUse this skill.',
      })),
      listMarketplaceSkills: vi.fn(async () => ({
        sources: [{ id: 'team-skills' }],
        skills: [skill],
        errors: [],
      })),
      listInstances: vi.fn(async () => ([
        { id: 'inst-1', name: 'Reviewer', status: 'live' },
      ])),
      listInstanceSkills: vi.fn(async () => []),
      installSkillToInstance: vi.fn(async () => ({
        installed: true,
        version: '1.0.0',
        sha256: 'computed',
      })),
    };

    render(
      <ApiProvider client={client} auth={{ mode: 'none' }}>
        <MarketplaceSkillDetailPage view={{ marketplace: 'team-skills', skill: 'code-review' }}/>
      </ApiProvider>,
    );

    expect(await screen.findByRole('heading', { name: 'code-review' })).toBeInTheDocument();
    expect(screen.getByText('declared')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Code Review' })).toBeInTheDocument();
    expect(screen.getByLabelText('skill markdown')).toHaveClass('skill-detail-markdown');
    expect(screen.getByLabelText('skill markdown')).not.toHaveStyle({ overflow: 'auto' });

    fireEvent.click(screen.getByRole('button', { name: 'Install to instance' }));
    expect(await screen.findByText('Reviewer')).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole('button', { name: 'Install selected' })).toBeEnabled());
    fireEvent.click(screen.getByRole('button', { name: 'Install selected' }));

    await waitFor(() => {
      expect(client.installSkillToInstance).toHaveBeenCalledWith('inst-1', {
        marketplace: 'team-skills',
        skill: 'code-review',
        force: false,
      });
    });
    expect(await screen.findByText('installed v1.0.0')).toBeInTheDocument();
  });

  test('instance skill inventory can uninstall a skill from the selected agent', async () => {
    const client = {
      uninstallSkillFromInstance: vi.fn(async () => ({ uninstalled: true, skill: 'code-review' })),
    };
    const onChanged = vi.fn();
    vi.spyOn(window, 'confirm').mockReturnValue(true);

    render(
      <ApiProvider client={client} auth={{ mode: 'none' }}>
        <SkillInventoryList
          instanceId="inst-1"
          onChanged={onChanged}
          rows={[{
            instance_id: 'inst-1',
            skill: 'code-review',
            description: 'Review code changes.',
            origin_kind: 'marketplace',
            marketplace_id: 'team-skills',
            version: '1.0.0',
            installed_at: '2026-05-08T08:00:00Z',
            updated_at: 100,
            synced_at: 101,
            has_body: true,
            has_metadata: true,
            source_path: 'workspace/skills/code-review/SKILL.md',
          }]}
        />
      </ApiProvider>,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Uninstall code-review' }));

    await waitFor(() => {
      expect(client.uninstallSkillFromInstance).toHaveBeenCalledWith('inst-1', 'code-review');
    });
    expect(onChanged).toHaveBeenCalled();
  });

  test('instance skill inventory can publish an agent-authored skill explicitly', async () => {
    const refreshed = [{
      instance_id: 'inst-1',
      skill: 'debug-logs',
      description: 'Read logs before guessing.',
      origin_kind: 'local',
      marketplace_id: null,
      version: '0.1.0',
      updated_at: 100,
      synced_at: 102,
      has_body: true,
      has_metadata: true,
      source_path: 'workspace/skills/debug-logs/SKILL.md',
      public: true,
      public_marketplace_id: 'agent-inst-1',
    }];
    const client = {
      publishSkillFromInstance: vi.fn(async () => ({
        instance_id: 'inst-1',
        skill: 'debug-logs',
        public: true,
      })),
      unpublishSkillFromInstance: vi.fn(),
      listInstanceSkills: vi.fn(async () => refreshed),
      listMarketplaceSkills: vi.fn(async () => ({
        sources: [{ id: 'agent-inst-1' }],
        skills: [{ marketplace_id: 'agent-inst-1', name: 'debug-logs' }],
        errors: [],
      })),
    };
    const onChanged = vi.fn();
    vi.spyOn(window, 'confirm').mockReturnValue(true);

    render(
      <ApiProvider client={client} auth={{ mode: 'none' }}>
        <SkillInventoryList
          instanceId="inst-1"
          onChanged={onChanged}
          rows={[{
            instance_id: 'inst-1',
            skill: 'debug-logs',
            description: 'Read logs before guessing.',
            origin_kind: 'local',
            marketplace_id: null,
            version: '0.1.0',
            updated_at: 100,
            synced_at: 101,
            has_body: true,
            has_metadata: true,
            source_path: 'workspace/skills/debug-logs/SKILL.md',
            public: false,
          }]}
        />
      </ApiProvider>,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Publish debug-logs' }));

    await waitFor(() => {
      expect(client.publishSkillFromInstance).toHaveBeenCalledWith('inst-1', 'debug-logs');
    });
    expect(client.listMarketplaceSkills).toHaveBeenCalled();
    expect(onChanged).toHaveBeenCalledWith(refreshed);
  });
});
