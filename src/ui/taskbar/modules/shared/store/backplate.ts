import { Settings } from "@magic-ui/lib";
import { TaskbarSettings } from "@magic-ui/types";
import { IconPackManager } from "@magic-ui/lib";

/**
 * 切换图标背板风格
 * @param style 背板风格: 'Transparent' 或 'White'
 */
export async function switchIconBackplateStyle(style: 'Transparent' | 'White'): Promise<void> {
  // 获取当前设置
  const currentSettings = await Settings.getAsync();

  // 更新背板风格
  currentSettings.magicTaskbar.iconBackplateStyle = style;

  // 保存设置
  await currentSettings.save();

  // 清除所有图标缓存，确保切换背板后显示正确的图标
  await IconPackManager.clearCachedIcons();

  // 日志记录
  console.log(`Icon backplate style switched to: ${style}, all icon caches cleared`);
}

/**
 * 获取当前图标背板风格
 */
export async function getCurrentIconBackplateStyle(): Promise<'Transparent' | 'White'> {
  const settings = await Settings.getAsync();
  return settings.magicTaskbar.iconBackplateStyle;
}
