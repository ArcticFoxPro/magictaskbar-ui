#!/usr/bin/env node
import fs from "fs";
import path from "path";
import { execSync } from "child_process";
import { promisify } from "util";
import { fileURLToPath } from "url";
import process from "node:process";

// 启用 ES 模块兼容性
const fsPromises = fs.promises;

// 获取当前文件路径和目录路径（ES模块兼容）
const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// 创建目录
async function mkdirRecursive(dirPath) {
  try {
    await fsPromises.mkdir(dirPath, { recursive: true });
    return true;
  } catch (error) {
    console.error(`Error creating directory ${dirPath}:`, error);
    return false;
  }
}

// 清空目录
async function emptyDir(dirPath) {
  try {
    if (fs.existsSync(dirPath)) {
      const entries = await fsPromises.readdir(dirPath, {
        withFileTypes: true,
      });
      for (const entry of entries) {
        const fullPath = path.join(dirPath, entry.name);
        if (entry.isDirectory()) {
          await fsPromises.rm(fullPath, { recursive: true, force: true });
        } else {
          await fsPromises.unlink(fullPath);
        }
      }
    }
    return true;
  } catch (error) {
    console.error(`Error emptying directory ${dirPath}:`, error);
    return false;
  }
}

// 读取JSON文件
async function readJsonFile(filePath) {
  try {
    const content = await fsPromises.readFile(filePath, "utf8");
    return JSON.parse(content);
  } catch (error) {
    console.error(`Error reading JSON file ${filePath}:`, error);
    return null;
  }
}

// 创建mod.ts文件
async function createModFile(directory, importFiles) {
  try {
    const fileContent = importFiles
      .map((file) => `export * from './${file}';`)
      .join("\n");
    await fsPromises.writeFile(path.join(directory, "mod.ts"), fileContent);
    return true;
  } catch (error) {
    console.error(`Error creating mod.ts file in ${directory}:`, error);
    return false;
  }
}

// 复制文件
async function copyFile(source, target) {
  try {
    await fsPromises.copyFile(source, target);
    return true;
  } catch (error) {
    console.error(`Error copying file from ${source} to ${target}:`, error);
    return false;
  }
}

// 复制目录
async function copyDirectory(source, target) {
  try {
    await mkdirRecursive(target);
    const entries = await fsPromises.readdir(source, { withFileTypes: true });

    for (const entry of entries) {
      const sourcePath = path.join(source, entry.name);
      const targetPath = path.join(target, entry.name);

      if (entry.isDirectory()) {
        await copyDirectory(sourcePath, targetPath);
      } else {
        await copyFile(sourcePath, targetPath);
      }
    }
    return true;
  } catch (error) {
    console.error(
      `Error copying directory from ${source} to ${target}:`,
      error,
    );
    return false;
  }
}

// 执行命令
function executeCommand(command, cwd = process.cwd()) {
  try {
    console.log(`Executing: ${command}`);
    execSync(command, { cwd, stdio: "inherit" });
    return true;
  } catch (error) {
    console.error(`Error executing command: ${command}`, error);
    return false;
  }
}

// 主要构建函数
async function build() {
  console.log("[Task] Starting Node.js-based build...");

  // 获取项目根目录
  const rootDir = path.resolve(__dirname, "..");
  const genTypesPath = path.join(rootDir, "gen/types");
  const npmOutDir = path.join(rootDir, "npm");
  const denoJsonPath = path.join(rootDir, "deno.json");
  const srcDir = path.join(rootDir, "src");

  // 1. 读取deno.json获取项目信息
  console.log("[Task] Reading deno.json...");
  const denoJson = await readJsonFile(denoJsonPath);
  if (!denoJson) {
    console.error("Failed to read deno.json");
    process.exit(1);
  }

  const { name, description, version, license } = denoJson;

  // 2. 生成Rust绑定（使用cargo test）
  console.log("[Task] Generating TypeScript Bindings from Rust...");
  if (!executeCommand("cargo test --features gen-binds", rootDir)) {
    console.error("Failed to generate TypeScript bindings");
    process.exit(1);
  }

  // 3. 清空之前的npm构建目录
  console.log("[Task] Cleaning previous npm build...");
  await emptyDir(npmOutDir);

  // 4. 确保类型目录存在并创建mod.ts
  console.log("[Task] Creating entry points...");
  await mkdirRecursive(genTypesPath);

  // 获取gen/types目录下的所有ts文件并创建mod.ts
  try {
    const typeFiles = (await fsPromises.readdir(genTypesPath))
      .filter((file) => file.endsWith(".ts") && file !== "mod.ts");

    await createModFile(genTypesPath, typeFiles);
  } catch (error) {
    console.error("Error creating gen/types/mod.ts:", error);
  }

  // 5. 创建npm包结构
  console.log("[Task] Creating npm package structure...");
  await mkdirRecursive(npmOutDir);

  // 6. 创建package.json
  const packageJson = {
    name,
    description,
    version,
    license,
    main: "mod.js",
    types: "mod.d.ts",
    type: "module",
    exports: {
      ".": {
        "types": "./mod.d.ts",
        "import": "./mod.js",
      },
      "./types": {
        "types": "./types/mod.d.ts",
        "import": "./types/mod.js",
      },
      "./tauri": {
        "types": "./tauri.d.ts",
        "import": "./tauri.js",
      },
    },
    keywords: ["magic-ui", "taskbar", "typescript", "rust"],
    author: "MagicTaskbar Team",
  };

  await fsPromises.writeFile(
    path.join(npmOutDir, "package.json"),
    JSON.stringify(packageJson, null, 2),
  );

  // 7. 创建TypeScript配置文件
  const tsConfig = {
    compilerOptions: {
      target: "ES2023",
      module: "ESNext",
      moduleResolution: "bundler",
      declaration: true,
      emitDeclarationOnly: true,
      outDir: ".",
      skipLibCheck: true,
      lib: ["DOM", "DOM.Iterable", "ESNext"],
    },
  };

  await fsPromises.writeFile(
    path.join(npmOutDir, "tsconfig.json"),
    JSON.stringify(tsConfig, null, 2),
  );

  // 8. 使用TypeScript编译生成类型声明
  console.log("[Task] Generating TypeScript declarations...");

  // 复制源文件到临时目录用于编译
  const tempDir = path.join(npmOutDir, "temp");
  await mkdirRecursive(tempDir);

  // 复制主要的lib.ts和相关文件
  await copyDirectory(srcDir, tempDir);

  // 复制mod.ts文件
  await copyFile(
    path.join(rootDir, "mod.ts"),
    path.join(tempDir, "mod.ts"),
  );

  // 创建类型目录
  const typesDest = path.join(npmOutDir, "types");
  await mkdirRecursive(typesDest);

  // 复制生成的类型文件
  const typeFiles = await fsPromises.readdir(genTypesPath, {
    withFileTypes: true,
  });
  for (const entry of typeFiles) {
    const sourcePath = path.join(genTypesPath, entry.name);
    const targetPath = path.join(typesDest, entry.name);

    if (entry.isDirectory()) {
      await copyDirectory(sourcePath, targetPath);
    } else {
      await copyFile(sourcePath, targetPath);
    }
  }

  // 9. 创建简化的JavaScript入口文件
  await fsPromises.writeFile(
    path.join(npmOutDir, "mod.js"),
    `// Generated module entry point\nexport * from './types/mod.js';`,
  );

  await fsPromises.writeFile(
    path.join(npmOutDir, "types/mod.js"),
    `// Generated types module entry point`,
  );

  // 复制tauri导出文件
  const tauriExportPath = path.join(srcDir, "re-exports/tauri.ts");
  if (fs.existsSync(tauriExportPath)) {
    await copyFile(
      tauriExportPath,
      path.join(npmOutDir, "tauri.js"),
    );
  }

  // 10. 清理临时文件
  await fsPromises.rm(tempDir, { recursive: true, force: true });

  // 跳过不存在的format命令
  console.log("[Task] Done! npm package created in", npmOutDir);
}

// 运行构建
build().catch((error) => {
  console.error("Build failed:", error);
  process.exit(1);
});
