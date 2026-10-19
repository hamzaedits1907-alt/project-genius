// Views that exist only in some builds (the browser build registers Code and Admin here). The shared App
// renders whatever is registered for the current route, so the desktop bundle never contains them.
import type { ComponentType } from 'react';

export const extraViews: Record<string, ComponentType> = {};
