import { describe, expect, test } from 'vitest';

import { InstancesView } from './instances.jsx';
import { renderView } from './app.jsx';

describe('renderView', () => {
  test('routes the Channels instance section into the instance shell', () => {
    const element = renderView({ name: 'instance-channels', id: 'dancing-horizon-846-4b26de' });

    expect(element.type).toBe(InstancesView);
    expect(element.props.view).toEqual({
      name: 'instance-channels',
      id: 'dancing-horizon-846-4b26de',
    });
  });
});
