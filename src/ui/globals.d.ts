﻿declare module "react-dom/server" {
  export function renderToStaticMarkup(element: any): string;
}

declare module "*.module.css" {
  const classnames: Record<string, string>;
  export default classnames;
}

declare module "*.module.scss" {
  const classnames: Record<string, string>;
  export default classnames;
}

declare module "*.yml" {
  export default string;
}

declare module "*.svg" {
  const src: string;
  export default src;
}

interface ObjectConstructor {
  keys<T>(o: T): (T extends any ? keyof T : PropertyKey)[];
}

interface Window {
  __TAURI_INTERNALS__: {
    metadata?: {
      currentWebview?: {
        label?: string;
      };
    };
    invoke: any;
  };
  __SLU_WIDGET: import("@magic-ui/lib/types").Widget;
}
