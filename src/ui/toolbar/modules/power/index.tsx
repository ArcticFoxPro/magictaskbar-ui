import { useEffect, useState } from "react";

import "./styles.css";

interface BatteryManager extends EventTarget {
  charging: boolean;
  chargingTime: number;
  dischargingTime: number;
  level: number;
  onchargingchange: ((this: BatteryManager, ev: Event) => any) | null;
  onchargingtimechange: ((this: BatteryManager, ev: Event) => any) | null;
  ondischargingtimechange: ((this: BatteryManager, ev: Event) => any) | null;
  onlevelchange: ((this: BatteryManager, ev: Event) => any) | null;
}

interface NavigatorWithBattery extends Navigator {
  getBattery: () => Promise<BatteryManager>;
}

// 根据电量百分比获取对应的图标等级 (1-10)
const getBatteryLevel = (pct: number): number => {
  if (pct >= 91) return 10;
  if (pct >= 81) return 9;
  if (pct >= 71) return 8;
  if (pct >= 61) return 7;
  if (pct >= 51) return 6;
  if (pct >= 41) return 5;
  if (pct >= 31) return 4;
  if (pct >= 21) return 3;
  if (pct >= 11) return 2;
  return 1;
};

export function PowerModule() {
  const [level, setLevel] = useState<number | null>(null);
  const [charging, setCharging] = useState<boolean>(false);
  const [supported, setSupported] = useState<boolean>(true);

  useEffect(() => {
    const nav = navigator as unknown as NavigatorWithBattery;
    if (!nav.getBattery) {
      setSupported(false);
      return;
    }

    let battery: BatteryManager | null = null;

    const updateBattery = () => {
      if (battery) {
        setLevel(battery.level);
        setCharging(battery.charging);
      }
    };

    nav.getBattery().then((bat) => {
      battery = bat;
      updateBattery();

      bat.addEventListener("levelchange", updateBattery);
      bat.addEventListener("chargingchange", updateBattery);
    });

    return () => {
      if (battery) {
        battery.removeEventListener("levelchange", updateBattery);
        battery.removeEventListener("chargingchange", updateBattery);
      }
    };
  }, []);

  if (!supported || level === null) {
    return null;
  }

  const pct = Math.round(level * 100);
  const batteryLevel = getBatteryLevel(pct);
  
  // 根据充电状态选择对应的 SVG 文件
  const iconSrc = charging 
    ? `/static/icons/BatteryAC${batteryLevel}.svg`
    : `/static/icons/Battery${batteryLevel}.svg`;

  return (
    <div className="taskbar-item taskbar-module power-module">
      <div className="power-content">
        {level !== null && (
          <div className="power-percentage">
            {pct}%
          </div>
        )}
        <div className={`power-icon ${charging ? "power-charging" : ""}`}>
          <div className="power-icon-container">
            <img
              src={iconSrc}
              alt={`Battery ${pct}%${charging ? " charging" : ""}`}
              width="20"
              height="20"
              className={`power-battery-icon ${charging ? "is-charging" : ""}`}
            />
          </div>
        </div>
      </div>
    </div>
  );
}
