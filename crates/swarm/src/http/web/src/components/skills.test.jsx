import { afterEach, describe, expect, test } from 'vitest';
import React from 'react';
import { cleanup, render, screen } from '@testing-library/react';
import '@testing-library/jest-dom/vitest';

import { SkillCatalogRow } from './skills.jsx';

afterEach(() => {
  cleanup();
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
    expect(screen.getByRole('link', { name: 'by Axelrod' }))
      .toHaveAttribute('href', '#/i/axelrod/skills');
  });
});
