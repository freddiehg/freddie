import type { SidebarsConfig } from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  documentationSidebar: [
    'index',
    'getting-started-with-mercury',
    'implementing-your-own-handler',
    'connecting-a-new-source-of-events',
    'adding-an-effect',
    {
      type: 'category',
      label: 'Architecture',
      items: [
        'architecture/index',
        'architecture/the-event-loop',
        'architecture/the-data-model',
        'architecture/dispatch-and-precedence',
        'architecture/typed-paths',
        'architecture/virtual-fields',
        'architecture/the-crates',
      ],
    },
    {
      type: 'category',
      label: 'Interacting with macOS',
      items: [
        'interacting-with-macos/index',
        'interacting-with-macos/grabbing-the-keyboard',
        'interacting-with-macos/emitting-keys',
        'interacting-with-macos/apps-and-the-frontmost-app',
        'interacting-with-macos/placing-windows',
        'interacting-with-macos/the-menu-bar-and-the-overlay',
        'interacting-with-macos/the-chrome-extension',
        'interacting-with-macos/logging',
      ],
    },
  ],
};

export default sidebars;
