import { cx } from "@shared/styles";
import { HideMode } from "@magic-ui/lib";
import { $bar_should_be_hidden, $settings, $has_maximized_window, $is_toolbar_overlaped } from "../shared/state/mod";
import { DateModule } from "../date";
import { PowerModule } from "../power";
import { NetworkModule } from "../network/index.tsx";
import { VolumeModule } from "../volume";
import { ControlCenterModule } from "../controlcenter";
import YoyoModule from "../yoyo";
import InputMethodModule from "../input-method";
import { NotificationModule } from "../notification";
import { BluetoothModule } from "../bluetooth";

import PowerMenuModule from "../power-menu";
import AIRecommendModule from "../ai-recommend";
import ShortcutModule from "../shortcut";
import { ToolbarContextMenuTrigger } from "./ContextMenu";
import { useSignalEffect } from "@preact/signals";

export function FancyToolbar() {
  // Listen for background style changes
  useSignalEffect(() => {
    console.info(
      '[Toolbar] Background style state: has_maximized_window=',
      $has_maximized_window.value,
      'is_toolbar_overlaped=',
      $is_toolbar_overlaped.value,
    );
  });
  
  return (
      <ToolbarContextMenuTrigger>
        <div
          className={cx("ft-bar", $settings.value.position.toLowerCase(), { 
              "ft-bar-hidden": $bar_should_be_hidden.value, 
              "ft-bar-has-maximized": 
                ($settings.value.hideMode === HideMode.Never && $has_maximized_window.value) || 
                ($settings.value.hideMode === HideMode.OnOverlap && $is_toolbar_overlaped.value), 
            })}
        >
          <div className="ft-bar-left"><PowerMenuModule /><AIRecommendModule /></div>
          <div className="ft-bar-center"></div>
          <div className="ft-bar-right">
            <ShortcutModule />
            <InputMethodModule />
            <BluetoothModule />
            <NetworkModule />
            <VolumeModule />
            <PowerModule />
            <ControlCenterModule />
            <YoyoModule />
            <DateModule />
            <NotificationModule />
          </div>
        </div>
      </ToolbarContextMenuTrigger>
    
  );
}
