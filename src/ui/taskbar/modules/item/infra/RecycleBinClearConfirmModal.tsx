import { useState } from "react";
import "./RecycleBinClearConfirmModal.styles.css";

// 回收站图标组件（SVG）- 使用相对路径
const RecycleBinIcon = () => {
  console.log('[RecycleBinIcon] Rendering icon component');
  return (
    <img
      src="../static/icons/RecycleBin_NotEmpty.svg"
      alt="回收站"
      className="recycle-bin-clear-modal-icon"
      onLoad={(e) => {
        console.log('[RecycleBinIcon] Image loaded successfully!');
      }}
      onError={(e) => {
        console.error('[RecycleBinIcon] Image failed to load:', (e.target as HTMLImageElement).src);
        console.error('[RecycleBinIcon] Element:', e.target);
        // 调试：临时添加红色边框
        (e.target as HTMLImageElement).style.border = '5px solid red';
        (e.target as HTMLImageElement).style.display = 'block';
      }}
    />
  );
};

interface RecycleBinClearConfirmModalProps {
  isOpen: boolean;
  itemCount: number;
  onConfirm: () => void;
  onCancel: () => void;
}

export function RecycleBinClearConfirmModal({
  isOpen,
  itemCount,
  onConfirm,
  onCancel
}: RecycleBinClearConfirmModalProps) {
  if (!isOpen) return null;

  const handleConfirm = () => {
    onConfirm();
  };

  const handleCancel = () => {
    onCancel();
  };

  return (
    <div className="recycle-bin-clear-modal-container" onClick={(e) => e.stopPropagation()}>
      <div className="recycle-bin-clear-modal-body">
        {/* 回收站图标 */}
        <RecycleBinIcon />

        {/* 文本框容器 - 包含提示文字和警告文字 */}
        <div className="recycle-bin-clear-modal-text-container">
          {/* 提示文字 */}
          <div className="recycle-bin-clear-modal-message">
            确定要永久清空回收站中的这 <span className="item-count">{itemCount}</span> 项吗？
          </div>

          {/* 警告文字 */}
          <div className="recycle-bin-clear-modal-warning">
            此操作无法撤销。
          </div>
        </div>

        {/* 按钮组 - macOS 风格 */}
        <div className="recycle-bin-clear-modal-buttons">
          <button
            className="recycle-bin-clear-modal-btn cancel-btn"
            onClick={handleCancel}
          >
            取消
          </button>

          <button
            className="recycle-bin-clear-modal-btn confirm-btn"
            onClick={handleConfirm}
          >
            清空回收站
          </button>
        </div>
      </div>
    </div>
  );
}

export default RecycleBinClearConfirmModal;
