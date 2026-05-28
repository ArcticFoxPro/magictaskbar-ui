import esbuild from "esbuild";
import CssModulesPlugin from "esbuild-css-modules-plugin";
import express from "express";
import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";
import { renderToStaticMarkup } from "react-dom/server"; // preact compat doesn't work for extracting icons
import yargs from "yargs";
import { hideBin } from "yargs/helpers";
import process from "node:process";

// 获取当前目录路径（ES模块兼容）
const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

async function getArgs() {
  const argv = await yargs(hideBin(process.argv))
    .option("production", {
      type: "boolean",
      description: "Enable Production Minified Bundle",
      default: false,
    })
    .option("serve", {
      type: "boolean",
      description: "Run a local server",
      default: false,
    }).argv;
  return {
    isProd: !!argv.production,
    serve: !!argv.serve,
  };
}

async function extractIconsIfNecessary() {
  if (fs.existsSync("./dist/icons")) {
    return;
  }

  console.info("Extracting SVG Lazy Icons");
  console.time("Lazy Icons");
  fs.mkdirSync("./dist/icons", { recursive: true });

  let tsFile = "// This file is generated on build, do not edit.\nexport type IconName =";
  const entries = fs.readdirSync("./node_modules/react-icons");

  for (const entry of entries) {
    const entryPath = path.join("./node_modules/react-icons", entry);
    const isDir = fs.statSync(entryPath).isDirectory();

    if (!isDir || entry === "lib") {
      continue;
    }

    console.info("Extracting icon family:", entry);

    const family = await import(`react-icons/${entry}`);
    for (const [name, ElementConstructor] of Object.entries(family)) {
      if (typeof ElementConstructor !== "function") {
        continue;
      }
      try {
        const element = ElementConstructor({ size: "1em" });
        const svg = renderToStaticMarkup(element);
        if (!svg.startsWith("<svg")) {
          console.warn(`Skipping invalid SVG for ${name}:`, svg);
          continue;
        }
        fs.writeFileSync(`./dist/icons/${name}.svg`, svg);
      } catch (error: unknown) {
        const msg = (error as any)?.message ?? String(error);
        console.warn(`Failed to process icon ${name}:`, msg);
        continue;
      }
    }

    tsFile += `\n  | keyof typeof import('react-icons/${entry}')`;
  }

  tsFile += ";\n";
  fs.writeFileSync("./libs/widgets-shared/components/Icon/icons.ts", tsFile);
  console.timeEnd("Lazy Icons");
}

const appFolders = fs
  .readdirSync("src/ui")
  .filter((item) => item !== "shared" && fs.statSync(path.join("src/ui", item)).isDirectory());

const entryPoints = appFolders
  .map((folder) => {
    const vanilla = `./src/ui/${folder}/index.ts`;
    const react = `./src/ui/${folder}/index.tsx`;
    const svelte = `./src/ui/${folder}/index.svelte`;
    if (fs.existsSync(vanilla)) {
      return vanilla;
    }
    if (fs.existsSync(react)) {
      return react;
    }
    if (fs.existsSync(svelte)) {
      return svelte;
    }
    return "";
  })
  .filter((file) => !!file);

entryPoints.push("./libs/widgets-integrity/mod.ts");

const SharedAliasPlugin: esbuild.Plugin = {
  name: "shared-alias",
  setup(build) {
    // Handle @shared (exact match)
    build.onResolve({ filter: /^@shared$/ }, (args) => {
      return { path: path.resolve(__dirname, '../libs/widgets-shared/index.ts') };
    });
    // Handle @shared/* (subpath imports)
    build.onResolve({ filter: /^@shared\// }, (args) => {
      const subpath = args.path.replace('@shared/', '');
      const basePath = path.resolve(__dirname, '../libs/widgets-shared', subpath);

      // Check if it's a directory first
      if (fs.existsSync(basePath) && fs.statSync(basePath).isDirectory()) {
        // Try common index files in the directory
        const indexFiles = ['index.ts', 'index.tsx', 'infra.ts', 'infra.tsx'];
        for (const indexFile of indexFiles) {
          const fullPath = path.join(basePath, indexFile);
          if (fs.existsSync(fullPath)) {
            return { path: fullPath };
          }
        }
      }

      // Try adding extensions to the base path
      const extensions = ['.ts', '.tsx'];
      for (const ext of extensions) {
        const fullPath = basePath + ext;
        if (fs.existsSync(fullPath)) {
          return { path: fullPath };
        }
      }

      // Fallback to original path
      return { path: basePath };
    });
  },
};

const OwnPlugin: esbuild.Plugin = {
  name: "copy-public-by-entry",
  setup(build) {
    build.onStart(() => {
      console.time("build");
    });
    build.onEnd(() => {
      // copy static folder to dist
      const staticPath = "src/static";
      const distStaticPath = "dist/static";
      if (fs.existsSync(staticPath)) {
        fs.cpSync(staticPath, distStaticPath, { recursive: true, force: true });
      }

      // copy public folder for each widget
      appFolders.forEach((folder) => {
        let source = `src/ui/${folder}/public`;
        let target = `dist/${folder}`;
        fs.cpSync(source, target, { recursive: true, force: true });
      });

      // move nested folders to root
      const nestedPath = "dist/src/ui";
      if (fs.existsSync(nestedPath)) {
        fs.readdirSync(nestedPath).forEach((folder) => {
          let source = `dist/src/ui/${folder}`;
          let target = `dist/${folder}`;
          fs.cpSync(source, target, { recursive: true, force: true });
        });
        fs.rmSync("dist/src", { recursive: true, force: true });
      }

      console.timeEnd("build");
    });
  },
};

function startDevServer() {
  const app = express();
  app.use(express.static("dist"));
  app.listen(35790, () => {
    console.info("Listening on http://localhost:35790");
  });
}

(async function main() {
  const { isProd, serve } = await getArgs();
  console.info(`isProd: ${isProd}, serve: ${serve}`);

  await extractIconsIfNecessary();

  console.info("Removing old artifacts");
  // delete all in dist less icons
  fs.readdirSync("dist").forEach((folder) => {
    if (folder !== "icons") {
      fs.rmSync(path.join("dist", folder), { recursive: true, force: true });
    }
  });

  const ctx = await esbuild.context({
    entryPoints: entryPoints,
    bundle: true,
    minify: isProd,
    sourcemap: !isProd,
    treeShaking: true,
    format: "esm",
    outdir: "./dist",
    jsx: "automatic",
    loader: {
      ".yml": "text",
      ".svg": "file",
      ".png": "file",
      ".ico": "file",
    },
    plugins: [
      SharedAliasPlugin,
      CssModulesPlugin({
        localsConvention: "camelCase",
        pattern: "do-not-use-on-themes-[local]-[hash]",
      }),
      OwnPlugin,
    ],
    alias: {
      react: "./node_modules/preact/compat/",
      "react/jsx-runtime": "./node_modules/preact/jsx-runtime",
      "react-dom": "./node_modules/preact/compat/",
      "react-dom/*": "./node_modules/preact/compat/*",
      "@magic-ui/lib": path.resolve(__dirname, '../libs/core/src/lib.ts'),
      "@magic-ui/lib/tauri": path.resolve(__dirname, '../libs/core/src/re-exports/tauri.ts'),
      "@magic-ui/types": path.resolve(__dirname, '../libs/core/gen/types/mod.ts'),
      "libs/*": path.resolve(__dirname, '../libs/*'),
    },
  });

  if (serve) {
    await ctx.watch();
    startDevServer();
  } else {
    await ctx.rebuild();
    await ctx.dispose();
  }
})();
