#pragma once

#include <QString>

// Keep editor chrome achromatic (R = G = B), including focus and selection.
// Hue belongs to media roles and warning states, never the workspace surfaces.
inline QString editorStyleSheet() {
    return QStringLiteral(R"QSS(
* {
    color: #d7d7d7;
    font-size: 13px;
}

QWidget {
    background: transparent;
}
QMainWindow,
QWidget#workspaceRoot {
    background: #0b0b0b;
}
QLabel {
    background: transparent;
}
QDialog { background: #181818; }
QToolTip {
    color: #e6e6e6;
    background: #272727;
    border: 1px solid #4d4d4d;
    padding: 5px 8px;
}
QComboBox QAbstractItemView {
    background: #1d1d1d;
    color: #d7d7d7;
    border: 1px solid #494949;
    selection-background-color: #484848;
    selection-color: #f5f5f5;
}

QMenuBar {
    background: #111111;
    border-bottom: 1px solid #272727;
    padding: 4px 9px;
}
QMenuBar::item {
    padding: 5px 10px;
    border-radius: 5px;
}
QMenuBar::item:selected { background: #272727; }
QMenu {
    background: #1b1b1b;
    border: 1px solid #303030;
    padding: 4px;
}
QMenu::item {
    padding: 7px 28px 7px 12px;
    border-radius: 4px;
}
QMenu::item:selected { background: #3d3d3d; }

QToolBar {
    background: #111111;
    border: 0;
    border-bottom: 1px solid #272727;
    spacing: 6px;
    padding: 6px 10px;
}
QToolBar::separator {
    background: #303030;
    width: 1px;
    margin: 4px 3px;
}

QWidget#panel,
QFrame#mediaShelf,
QFrame#viewerFrame,
QWidget#timelinePanel {
    background: #181818;
    border: 1px solid #282828;
    border-radius: 10px;
}
QFrame#timelineFrame {
    background: transparent;
    border: 0;
}
QWidget#transport {
    background: #202020;
    border: 0;
    border-radius: 6px;
}
QFrame#agentContext {
    background: #1d1d1d;
    border: 1px solid #303030;
    border-radius: 8px;
}
QSplitter::handle { background: #0b0b0b; border-radius: 3px; }
QSplitter::handle:hover { background: #4e4e4e; }
QSplitter::handle:horizontal { width: 8px; }
QSplitter::handle:vertical { height: 8px; }

QToolButton,
QPushButton {
    background: #272727;
    border: 1px solid #333333;
    border-radius: 6px;
    padding: 4px 9px;
    min-height: 20px;
}
QToolBar QToolButton {
    background: transparent;
    border-color: transparent;
    min-width: 24px;
    padding: 3px 7px;
}
QToolBar QToolButton:hover,
QToolButton:hover,
QPushButton:hover {
    background: #303030;
    border-color: #5b5b5b;
}
QToolBar QToolButton:pressed,
QToolButton:pressed,
QPushButton:pressed {
    background: #4a4a4a;
    border-color: #747474;
}
QToolButton:focus,
QPushButton:focus {
    border-color: #828282;
}
QToolBar QToolButton:disabled,
QToolButton:disabled,
QPushButton:disabled {
    color: #727272;
    background: #1b1b1b;
    border-color: #272727;
}
QPushButton#primaryButton,
QPushButton#chatSendButton {
    background: #c8c8c8;
    border-color: #dbdbdb;
    color: #1c1c1c;
    font-weight: 600;
}
QPushButton#primaryButton:hover,
QPushButton#chatSendButton:hover { background: #d9d9d9; }
QPushButton#primaryButton:disabled,
QPushButton#chatSendButton:disabled {
    color: #777777;
    background: #272727;
    border-color: #303030;
}
QPushButton#dangerButton { background: #353535; border-color: #535353; }
QPushButton#dangerButton:hover { background: #474747; }
QPushButton#secondaryButton { background: #1d1d1d; }
QToolButton#squareButton {
    background: transparent;
    border-color: transparent;
    padding: 2px 9px;
    font-size: 18px;
    color: #cbcbcb;
}
QToolButton#squareButton:hover { background: #393939; border-color: #494949; }
QToolButton#playButton {
    background: #c8c8c8;
    border-color: #dbdbdb;
    border-radius: 6px;
    color: #1c1c1c;
    font-size: 15px;
    padding: 4px 14px;
}
QToolButton#playButton:hover { background: #d9d9d9; }
QToolButton#playButton:disabled { background: #272727; color: #777777; border-color: #303030; }
QToolButton#mediaToggle {
    background: transparent;
    border: 0;
    color: #a2a2a2;
    padding: 1px 5px;
}
QToolButton#mediaToggle:hover { color: #e1e1e1; background: #2c2c2c; }
QToolButton#abortButton {
    color: #c5c5c5;
    background: transparent;
    border-color: transparent;
    padding: 5px 7px;
}
QToolButton#abortButton:hover { background: #2c2c2c; border-color: #4e4e4e; }
QToolButton#abortButton:disabled { color: #6e6e6e; background: transparent; border-color: transparent; }

QLabel#eyebrow { color: #a1a1a1; font-size: 12px; font-weight: 500; letter-spacing: 0px; }
QLabel#muted { color: #a5a5a5; font-size: 11px; }
QLabel#chatHint { color: #eeeeee; font-size: 19px; font-weight: 600; padding: 20px 4px 2px; }
QLabel#statusPill {
    color: #b3b3b3;
    background: #222222;
    border: 1px solid #303030;
    border-radius: 8px;
    padding: 2px 6px;
    font-size: 10px;
}
QLabel#timecode { color: #e2e2e2; font-family: Menlo, Monaco, monospace; font-size: 11px; }
QLabel#inspectorClip { color: #eeeeee; font-weight: 600; padding: 5px 0; }
QLabel#projectTitle { color: #e1e1e1; font-weight: 600; padding: 0 20px; }
QLabel#assistantTitle { color: #e9e9e9; font-size: 14px; font-weight: 600; }
QLabel#agentConfig { color: #cacaca; font-size: 11px; font-weight: 600; }
QLabel#proposalSummary { color: #bdbdbd; }
QToolButton#eyebrow { text-align: left; background: transparent; border: 0; color: #a1a1a1; }

QListWidget,
QTextEdit,
QLineEdit,
QDoubleSpinBox,
QComboBox {
    background: #141414;
    border: 1px solid #303030;
    border-radius: 6px;
    padding: 6px;
    selection-background-color: #484848;
    selection-color: #f5f5f5;
}
QListWidget:hover,
QTextEdit:hover,
QLineEdit:hover,
QDoubleSpinBox:hover,
QComboBox:hover { border-color: #484848; }
QListWidget:focus,
QTextEdit:focus,
QLineEdit:focus,
QDoubleSpinBox:focus,
QComboBox:focus {
    border-color: #828282;
    background: #1c1c1c;
}
QListWidget::item { padding: 4px; border: 0; border-radius: 5px; }
QListWidget::item:hover { background: #343434; }
QListWidget::item:selected { background: #484848; color: #f5f5f5; }
QListWidget#mediaList,
QListWidget#activityList { background: transparent; border: 0; padding: 0; }
QTextEdit#conversation { background: transparent; border: 0; padding: 4px; color: #d0d0d0; }
QTextEdit#chatInput {
    background: transparent;
    border: 0;
    padding: 0;
    color: #e7e7e7;
    font-size: 13px;
}
QTextEdit#chatInput:focus { background: transparent; border: 0; }
QListWidget:disabled,
QTextEdit:disabled,
QLineEdit:disabled,
QDoubleSpinBox:disabled,
QComboBox:disabled {
    color: #7b7b7b;
    background: #1d1d1d;
    border-color: #272727;
}
QComboBox#agentProvider,
QComboBox#agentModel {
    background: transparent;
    border: 0;
    padding: 3px 2px;
    font-size: 11px;
    color: #c3c3c3;
}
QComboBox#agentProvider:hover,
QComboBox#agentModel:hover { background: #393939; }
QComboBox#agentModel QLineEdit { background: transparent; border: 0; padding: 0; }
QFrame#chatComposer {
    background: #212121;
    border: 1px solid #333333;
    border-radius: 12px;
}
QFrame#chatComposer QWidget { background: transparent; }
QPushButton#chatSendButton { border-radius: 14px; padding: 0; font-size: 19px; }

QGroupBox#proposalBox,
QGroupBox#providerBox {
    border: 1px solid #484848;
    border-radius: 7px;
    margin-top: 12px;
    padding: 12px 10px 10px;
    background: #171717;
}
QGroupBox#providerBox { margin-top: 8px; padding: 10px 8px 8px; }
QGroupBox#proposalBox::title,
QGroupBox#providerBox::title {
    subcontrol-origin: margin;
    left: 9px;
    padding: 0 5px;
    color: #b2b2b2;
    font-size: 11px;
    font-weight: 600;
}
QProgressBar { background: #141414; border: 0; border-radius: 3px; height: 4px; }
QProgressBar::chunk { background: #999999; border-radius: 3px; }

QTabWidget::pane { border: 0; background: transparent; }
QTabBar { background: transparent; }
QTabBar::tab {
    color: #999999;
    background: transparent;
    padding: 7px 14px;
    border: 1px solid transparent;
    border-radius: 6px;
}
QTabBar::tab:hover { color: #d9d9d9; background: #313131; }
QTabBar::tab:selected {
    color: #f1f1f1;
    background: #2d2d2d;
    border-color: #424242;
}

QSlider::groove:horizontal { background: #101010; border: 1px solid #2c2c2c; height: 5px; border-radius: 3px; }
QSlider::handle:horizontal { background: #b3b3b3; border: 1px solid #d6d6d6; width: 11px; margin: -4px 0; border-radius: 6px; }
QScrollBar:vertical, QScrollBar:horizontal { background: #101010; border: 0; }
QScrollBar:vertical { width: 8px; }
QScrollBar:horizontal { height: 8px; }
QScrollBar::handle:vertical, QScrollBar::handle:horizontal { background: #4f4f4f; border-radius: 4px; min-height: 20px; min-width: 20px; }
QScrollBar::handle:vertical:hover, QScrollBar::handle:horizontal:hover { background: #6d6d6d; }
QScrollBar::add-line, QScrollBar::sub-line { width: 0; height: 0; }
QStatusBar { background: #111111; color: #9a9a9a; border-top: 1px solid #272727; font-size: 10px; }
QFrame#connectionBanner,
QFrame#errorBanner { background: #392f27; border: 1px solid #8f6b4c; border-radius: 6px; }
)QSS");
}
