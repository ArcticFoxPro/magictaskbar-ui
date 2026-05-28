import { cx } from "@shared/styles";
import { computed } from "@preact/signals";

import { SeparatorTaskbarItem } from "../../shared/store/domain";

import { HardcodedSeparator1, HardcodedSeparator2, $dock_state } from "../../shared/state/items";
import { $settings } from "../../shared/state/mod";

function getTaskbarRegions() {
  const items = $dock_state.value.items;
  const separator1Index = items.indexOf(HardcodedSeparator1);
  const separator2Index = items.indexOf(HardcodedSeparator2);

  const leftRegion = items.slice(0, separator1Index).filter(item => item.type !== "Separator");
  const centerRegion = items.slice(separator1Index + 1, separator2Index).filter(item => item.type !== "Separator");
  const rightRegion = items.slice(separator2Index + 1).filter(item => item.type !== "Separator");

  return { leftRegion, centerRegion, rightRegion, separator1Index, separator2Index };
}

function shouldShowSeparator1(regions: ReturnType<typeof getTaskbarRegions>) {
  // 当分隔符左边所有区域（左侧）为空，或右边所有区域（中间+右侧）为空时才隐藏
  const leftAllEmpty = regions.leftRegion.length === 0;
  const rightAllEmpty = regions.centerRegion.length === 0 && regions.rightRegion.length === 0;
  return !(leftAllEmpty || rightAllEmpty);
}

function shouldShowSeparator2(regions: ReturnType<typeof getTaskbarRegions>) {
  // 只有当中间区域和右侧区域同时有内容时，才显示第二个分隔符
  return regions.centerRegion.length > 0 && regions.rightRegion.length > 0;
}

export function Separator({ item }: { item: SeparatorTaskbarItem }) {
  const shouldShowHardcodedSeparator = computed(() => {
    if (item.id !== HardcodedSeparator1.id && item.id !== HardcodedSeparator2.id) {
      return true;
    }

    const regions = getTaskbarRegions();

    if (item.id === HardcodedSeparator1.id) {
      return shouldShowSeparator1(regions);
    }

    if (item.id === HardcodedSeparator2.id) {
      return shouldShowSeparator2(regions);
    }

    return true;
  });

  return (
    <div
      className={cx("taskbar-separator", {
        "taskbar-separator-1": item.id === HardcodedSeparator1.id,
        "taskbar-separator-2": item.id === HardcodedSeparator2.id,
        visible: $settings.value.visibleSeparators && shouldShowHardcodedSeparator.value,
      })}
    />
  );
}
