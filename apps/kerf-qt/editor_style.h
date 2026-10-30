#pragma once

#include <QString>

// Finishing-room surfaces: near-black wells, one amber accent for the playhead
// and focus, and brightness steps instead of extra hues. Density follows
// desktop edit suites: 12px type, docked panels with title strips, and a
// status bar.
inline QString editorStyleSheet() {
    return QStringLiteral(R"QSS(
* {
    color: #d7d7d7;
    font-size: 12px;
}

QWidget {
    background: transparent;
}
QMainWindow,
QWidget#workspaceRoot {
    background: #0e0e0e;
}
QLabel {
    background: transparent;
}
QDialog { background: #1c1c1c; }
QToolTip {
    color: #e6e6e6;
    background: #363636;
    border: 1px solid #4d4d4d;
    padding: 5px 8px;
}
QComboBox QAbstractItemView {
    background: #2a2a2a;
    color: #d7d7d7;
    border: 1px solid #494949;
    selection-background-color: #484848;
    selection-color: #f5f5f5;
}

QMenuBar {
    background: #141414;
    border-bottom: 1px solid #0a0a0a;
    padding: 2px 6px;
}
QMenuBar::item {
    padding: 3px 8px;
    border-radius: 3px;
}
QMenuBar::item:selected { background: #363636; }
QMenu {
    background: #1c1c1c;
    border: 1px solid #2e2e2e;
    padding: 3px;
}
QMenu::item {
    padding: 4px 24px 4px 10px;
    border-radius: 3px;
}
QMenu::separator { height: 1px; background: #2e2e2e; margin: 3px 6px; }
QMenu::item:selected { background: #3d3d3d; }

QToolBar {
    background: #141414;
    border: 0;
    border-bottom: 1px solid #0a0a0a;
    spacing: 2px;
    padding: 0px 6px;
}
QToolBar::separator {
    background: #404040;
    width: 1px;
    margin: 2px 3px;
}

QWidget#panel,
QWidget#assistantPanel,
QFrame#viewerFrame,
QWidget#timelinePanel {
    background: #1a1a1a;
    border: 0;
    border-radius: 0px;
}
QFrame#timelineFrame {
    background: transparent;
    border: 0;
}
QWidget#transport {
    background: #191919;
    border: 0;
    border-top: 1px solid #0a0a0a;
    border-radius: 0px;
}
QLabel#transportTime { color: #f0b429; font-family: Menlo, Monaco, monospace; font-size: 13px; font-weight: 600; }
QLabel#transportDuration { color: #7c7c7c; font-family: Menlo, Monaco, monospace; font-size: 11px; }
QSplitter::handle { background: #0a0a0a; }
QSplitter::handle:hover { background: #4e4e4e; }
QSplitter::handle:horizontal { width: 2px; }
QSplitter::handle:vertical { height: 2px; }

QToolButton,
QPushButton {
    background: #363636;
    border: 1px solid #444444;
    border-radius: 3px;
    padding: 3px 8px;
    min-height: 18px;
}
QToolBar QToolButton {
    background: transparent;
    border-color: transparent;
    min-width: 20px;
    min-height: 16px;
    padding: 0px 5px;
}
QToolBar QToolButton:hover,
QToolButton:hover,
QPushButton:hover {
    background: #404040;
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
    background: #242424;
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
    background: #363636;
    border-color: #303030;
}
QPushButton#dangerButton { background: #353535; border-color: #535353; }
QPushButton#dangerButton:hover { background: #474747; }
QPushButton#secondaryButton { background: #2a2a2a; }
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
    border-radius: 4px;
    color: #1c1c1c;
    font-size: 15px;
    padding: 4px 14px;
}
QToolButton#playButton:hover { background: #d9d9d9; }
QToolButton#playButton:disabled { background: #363636; color: #777777; border-color: #303030; }
QToolButton#transportButton,
QPushButton#transportButton {
    background: transparent;
    border: 1px solid transparent;
    border-radius: 3px;
    color: #d2d2d2;
    padding: 1px 5px;
    min-height: 18px;
    min-width: 18px;
    font-size: 11px;
}
QToolButton#transportButton:disabled { background: transparent; border-color: transparent; }
QToolButton#transportButton:hover,
QPushButton#transportButton:hover {
    background: #333333;
    border-color: #5a5a5a;
    color: #f2f2f2;
}
QToolBar QComboBox#qualityPicker,
QToolBar QComboBox#canvasPicker {
    background: #2a2a2a;
    border: 1px solid #343434;
    border-radius: 4px;
    padding: 0px 18px 0px 6px;
    min-height: 18px;
}
QToolBar QComboBox#qualityPicker:hover,
QToolBar QComboBox#canvasPicker:hover {
    background: #333333;
    border-color: #4a4a4a;
}
QToolBar QComboBox#qualityPicker::drop-down,
QToolBar QComboBox#canvasPicker::drop-down {
    border: 0;
    width: 18px;
}
QToolBar QToolButton#toolbarPrimary {
    background: #c8c8c8;
    border: 1px solid #dbdbdb;
    border-radius: 4px;
    color: #1c1c1c;
    font-weight: 600;
    padding: 0px 8px;
}
QToolBar QToolButton#toolbarPrimary:hover { background: #d9d9d9; }
QToolBar QToolButton#toolbarPrimary:disabled {
    color: #777777;
    background: #363636;
    border-color: #303030;
}
QToolBar QToolButton#toolbarQuiet {
    background: transparent;
    border: 1px solid transparent;
    color: #c8c8c8;
    padding: 0px 6px;
}
QToolBar QToolButton#toolbarQuiet:hover {
    background: #2c2c2c;
    border-color: #3a3a3a;
    color: #f2f2f2;
}
QToolBar QToolButton#toolbarQuiet:disabled {
    color: #6a6a6a;
    background: transparent;
    border-color: transparent;
}
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
QLabel#chatHint { color: #a8a8a8; font-size: 11px; padding: 6px 2px 2px; }
QLabel#statusPill {
    color: #b3b3b3;
    background: #222222;
    border: 1px solid #404040;
    border-radius: 4px;
    padding: 2px 6px;
    font-size: 10px;
}
QLabel#timecode { color: #e2e2e2; font-family: Menlo, Monaco, monospace; font-size: 11px; }
QLabel#inspectorClip { color: #eeeeee; font-weight: 600; padding: 5px 0; }
QLabel#projectTitle { color: #e1e1e1; font-weight: 600; padding: 0 20px; }
QWidget#assistantPanel { background: #161616; border-left: 1px solid #0c0c0c; }
QWidget#assistantPanel QLabel#statusPill {
    color: #e6c98a;
    background: #2a261c;
    border-color: #5c4b2e;
    border-radius: 3px;
    padding: 0px 5px;
    font-size: 9px;
}
QLabel#composerStatus { color: #a6a6a6; font-size: 10px; }
QLabel#chatActivity {
    color: #e6d3a8;
    background: #26231c;
    border-left: 2px solid #c6a15a;
    padding: 0 8px;
    font-size: 11px;
}
QLabel#proposalSummary { color: #bdbdbd; }
QToolButton#eyebrow { text-align: left; background: transparent; border: 0; color: #a1a1a1; }

QListWidget,
QTextEdit,
QLineEdit,
QDoubleSpinBox,
QComboBox {
    background: #2e2e2e;
    border: 1px solid #404040;
    border-radius: 3px;
    padding: 4px;
    selection-background-color: #484848;
    selection-color: #f5f5f5;
}
QListWidget:hover,
QTextEdit:hover,
QLineEdit:hover,
QDoubleSpinBox:hover,
QComboBox:hover { border-color: #484848; }
QComboBox#qualityPicker,
QComboBox#canvasPicker {
    padding-right: 30px;
}
QComboBox#qualityPicker::drop-down,
QComboBox#canvasPicker::drop-down {
    subcontrol-origin: padding;
    subcontrol-position: top right;
    width: 30px;
    background: transparent;
    border: 0;
    border-left: 1px solid #404040;
    border-top-right-radius: 4px;
    border-bottom-right-radius: 4px;
}
QListWidget:focus,
QTextEdit:focus,
QLineEdit:focus,
QDoubleSpinBox:focus,
QComboBox:focus {
    border-color: #828282;
    background: #363636;
}
QListWidget::item { padding: 4px; border: 0; border-radius: 4px; }
QListWidget::item:hover { background: #343434; }
QListWidget::item:selected { background: #484848; color: #f5f5f5; }
QListWidget#mediaList,
QListWidget#activityList,
QScrollArea#mediaList { background: transparent; border: 0; padding: 0; }
QWidget#mediaGrid { background: transparent; }
QTextEdit#conversation { background: transparent; border: 0; padding: 4px 0; color: #d0d0d0; }
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
    background: #2a2a2a;
    border-color: #272727;
}
QComboBox#agentModel {
    background: transparent;
    border: 0;
    padding: 3px 2px;
    font-size: 11px;
    color: #c3c3c3;
}
QComboBox#agentModel:hover { background: #393939; }
QComboBox#agentModel QLineEdit { background: transparent; border: 0; padding: 0; }
QFrame#chatComposer {
    background: #2e2e2e;
    border: 1px solid #444444;
    border-radius: 12px;
}
QFrame#chatComposer[state="focused"] { border-color: #c6a15a; background: #2a271f; }
QFrame#chatComposer[state="sending"] { border-color: #8a7040; background: #26231c; }
QFrame#chatComposer[state="offline"] { border-color: #3c3c3c; background: #272727; }
QTextEdit#chatInput:disabled { background: transparent; color: #999999; border: 0; }
QPushButton#chatSendButton { border-radius: 8px; padding: 0; font-size: 19px; }
QToolButton#abortButton { border: 1px solid #8a7040; border-radius: 7px; padding: 4px 9px; }

QGroupBox#proposalBox,
QGroupBox#providerBox,
QGroupBox#inboundMcpSection,
QGroupBox#mcpSection {
    border: 1px solid #484848;
    border-radius: 3px;
    margin-top: 10px;
    padding: 10px 8px 8px;
    background: #2a2a2a;
}
QGroupBox#providerBox,
QGroupBox#inboundMcpSection,
QGroupBox#mcpSection { margin-top: 8px; padding: 10px 8px 8px; }
QGroupBox#proposalBox::title,
QGroupBox#providerBox::title,
QGroupBox#inboundMcpSection::title,
QGroupBox#mcpSection::title {
    subcontrol-origin: margin;
    left: 9px;
    padding: 0 5px;
    color: #b2b2b2;
    font-size: 11px;
    font-weight: 600;
}
QProgressBar { background: #2e2e2e; border: 0; border-radius: 2px; height: 4px; max-height: 4px; }
QProgressBar::chunk { background: #c6a15a; border-radius: 3px; }

QTabWidget::pane { border: 0; background: transparent; }
QTabBar { background: transparent; }
QTabBar::tab {
    color: #999999;
    background: transparent;
    padding: 4px 10px;
    border: 1px solid transparent;
    border-radius: 4px;
}
QTabBar::tab:hover { color: #d9d9d9; background: #313131; }
QTabBar::tab:selected {
    color: #f1f1f1;
    background: #2d2d2d;
    border-color: #424242;
}

QSlider::groove:horizontal { background: #101010; border: 1px solid #2c2c2c; height: 5px; border-radius: 3px; }
QSlider::handle:horizontal { background: #b3b3b3; border: 1px solid #d6d6d6; width: 11px; margin: -4px 0; border-radius: 4px; }
QScrollBar:vertical, QScrollBar:horizontal { background: #101010; border: 0; }
QScrollBar:vertical { width: 8px; }
QScrollBar:horizontal { height: 8px; }
QScrollBar::handle:vertical, QScrollBar::handle:horizontal { background: #4f4f4f; border-radius: 4px; min-height: 20px; min-width: 20px; }
QScrollBar::handle:vertical:hover, QScrollBar::handle:horizontal:hover { background: #6d6d6d; }
QScrollBar::add-line, QScrollBar::sub-line { width: 0; height: 0; }
QStatusBar { background: #141414; color: #8e8e8e; border-top: 1px solid #0a0a0a; font-size: 10px; min-height: 20px; max-height: 20px; }
QStatusBar::item { border: 0; }
QStatusBar QLabel { color: #8e8e8e; font-size: 10px; padding: 0 8px; border-left: 1px solid #262626; }
QStatusBar QLabel#statusConnection[state="ready"] { color: #b9d7a6; }
QStatusBar QLabel#statusConnection[state="connecting"] { color: #e6c98a; }
QStatusBar QLabel#statusConnection[state="disconnected"] { color: #e0a0a0; }
QFrame#connectionBanner,
QFrame#errorBanner { background: #352b36; border: 1px solid #896985; border-radius: 4px; }
QToolBar#toolPalette { background: #171717; border: 0; border-right: 1px solid #0a0a0a; spacing: 2px; padding: 4px 2px; }
QToolBar#toolPalette QToolButton { color: #a9a9a9; font-size: 15px; min-width: 24px; min-height: 24px; border-radius: 3px; }
QToolBar#toolPalette QToolButton:hover { background: #2c2c2c; color: #e6e6e6; }
QToolBar#toolPalette QToolButton:checked { background: #3a3220; color: #f0b429; }
QListWidget#projectBin { background: #161616; border: 0; }
QListWidget#projectBin::item { color: #cfcfcf; border-radius: 3px; padding: 4px; }
QListWidget#projectBin::item:selected { background: #2f2f2f; color: #f0f0f0; }
QWidget#sourceMonitor { background: #1a1a1a; }
QFrame#askPrompt { background: #232323; border: 1px solid #f0b429; border-radius: 8px; }
QLabel#askTitle { color: #f0b429; font-size: 12px; font-weight: 600; }
QTextEdit#askInput { background: #161616; border: 1px solid #3a3a3a; border-radius: 5px; padding: 4px; color: #ededed; }
QFrame#aiBar { background: #2a2414; border: 0; border-bottom: 1px solid #5a4a1c; }
QLabel#aiBarText { color: #f3d58a; font-size: 11px; }
QFrame#aiBar QToolButton { color: #e6e6e6; padding: 2px 10px; border: 1px solid #5a4a1c; border-radius: 4px; }
QFrame#aiBar QToolButton:hover { background: #3a3220; }
QFrame#aiBar QToolButton#aiAccept { background: #f0b429; color: #141414; border-color: #f0b429; font-weight: 600; }
QPushButton#nextStep { color: #f3d58a; background: #2a2414; padding: 3px 10px; border: 1px solid #5a4a1c; border-radius: 11px; font-size: 11px; }
QPushButton#nextStep:hover { background: #3a3220; border-color: #f0b429; }
QFrame#aiBar QToolButton:disabled { color: #7a7a7a; background: transparent; border-color: #3a3a3a; }

QFrame#divergedBanner { background: #2a2414; border: 1px solid #5a4a1c; border-radius: 4px; }
QLabel#divergedMessage { color: #f3d58a; font-size: 11px; }
QPushButton#loadFromDisk { background: #f0b429; border-color: #f0b429; color: #141414; font-weight: 600; }
QPushButton#loadFromDisk:hover { background: #f5c24a; }
QStatusBar QToolButton#syncChip {
    background: transparent;
    border: 0;
    border-left: 1px solid #262626;
    border-radius: 0;
    color: #a9a9a9;
    font-size: 10px;
    padding: 0 8px;
    min-height: 0;
}
QStatusBar QToolButton#syncChip:hover { color: #f0f0f0; background: #1f1f1f; }
QStatusBar QToolButton#syncChip[state="local"] { color: #7c7c7c; }
QStatusBar QToolButton#syncChip[state="syncing"] { color: #e6c98a; }
QStatusBar QToolButton#syncChip[state="live"] { color: #b9d7a6; }
QStatusBar QToolButton#syncChip[state="conflict"],
QStatusBar QToolButton#syncChip[state="failed"] { color: #e0a0a0; }
QWidget#presetsPanel { background: #1a1a1a; }
QLabel#presetsTitle { color: #8e8e8e; font-size: 10px; font-weight: 600; }
QListWidget#presetList { background: #161616; border: 0; padding: 2px; }
QListWidget#presetList::item { padding: 6px; border-radius: 4px; }
QListWidget#presetList::item:selected { background: #2f2f2f; color: #f0f0f0; }
QToolButton#presetsRefresh { background: transparent; border: 0; padding: 0 5px; }

QDialog#projectLauncher { background: #181818; }
QListWidget#launcherSidebar {
    background: #121212;
    border: 0;
    border-right: 1px solid #0a0a0a;
    border-radius: 0;
    padding: 16px 8px;
}
QListWidget#launcherSidebar::item { color: #c4c4c4; padding: 7px 8px; margin: 1px 0; border-radius: 5px; }
QListWidget#launcherSidebar::item:hover { background: #202020; }
QListWidget#launcherSidebar::item:selected { background: #2a2414; color: #f3d58a; }
QWidget#launcherContent { background: #181818; }
QLabel#launcherHeading { color: #f0f0f0; font-size: 18px; font-weight: 600; }
QLabel#launcherHint { color: #8e8e8e; font-size: 11px; }
QLabel#launcherEyebrow { color: #9a9a9a; font-size: 10px; font-weight: 600; }
QListWidget#templateTiles,
QListWidget#recentProjects,
QListWidget#presetChoices { background: transparent; border: 0; padding: 0; }
QListWidget#templateTiles::item { color: #cfcfcf; border: 1px solid transparent; border-radius: 10px; padding: 6px; }
QListWidget#templateTiles::item:hover { background: #1f1f1f; }
QListWidget#templateTiles::item:selected { background: #2a2414; border-color: #f0b429; color: #f5f5f5; }
QListWidget#recentProjects::item { color: #d7d7d7; padding: 8px; border-radius: 6px; }
QListWidget#presetChoices::item { color: #d7d7d7; padding: 6px 8px; border-radius: 6px; }
QListWidget#recentProjects::item:selected,
QListWidget#presetChoices::item:selected { background: #2a2414; color: #f5f5f5; }
QToolButton#launcherDetailsToggle { background: transparent; border: 0; color: #d0d0d0; font-weight: 600; padding: 2px 0; }
QToolButton#launcherDetailsToggle:hover { background: transparent; color: #f2f2f2; }
QWidget#launcherDetailsBody { background: #1f1f1f; border: 1px solid #2c2c2c; border-radius: 8px; }
QFrame#launcherFooter { background: #1c1c1c; border: 0; border-top: 1px solid #0a0a0a; }
QLabel#launcherProblem { color: #9a9a9a; font-size: 11px; }
QLabel#launcherProblem[state="error"] { color: #efb3b8; }
QLabel#launcherProblem[state="busy"] { color: #e6c98a; }
QPushButton#launcherChoose { background: #f0b429; border-color: #f0b429; color: #141414; font-weight: 600; padding: 4px 16px; }
QPushButton#launcherChoose:hover { background: #f5c24a; border-color: #f5c24a; }
QPushButton#launcherChoose:disabled { color: #777777; background: #2a2a2a; border-color: #303030; }
QFrame#presetSlot { background: #1f1f1f; border: 1px solid #2c2c2c; border-radius: 6px; }
QLabel#presetSlotName { color: #eeeeee; font-weight: 600; }
QDialog#projectLauncher QCheckBox,
QDialog#publishDialog QCheckBox { spacing: 7px; }
QDialog#projectLauncher QCheckBox::indicator,
QDialog#publishDialog QCheckBox::indicator {
    width: 12px;
    height: 12px;
    background: #2a2a2a;
    border: 1px solid #5a5a5a;
    border-radius: 3px;
}
QDialog#projectLauncher QCheckBox::indicator:checked,
QDialog#publishDialog QCheckBox::indicator:checked { background: #f0b429; border-color: #f0b429; }
QDialog#projectLauncher QCheckBox::indicator:disabled,
QDialog#publishDialog QCheckBox::indicator:disabled { background: #202020; border-color: #333333; }
QDialog#projectLauncher QCheckBox:disabled { color: #6f6f6f; }
QLabel[fit="none"] { color: #8e8e8e; }
QLabel[fit="yes"] { color: #b9d7a6; }
QLabel[fit="no"] { color: #efb3b8; }
)QSS");
}

// Docked panels, layered over the dock manager's own stylesheet: dark title
// strips with the panel names as tabs and an amber line under the focused one.
inline QString dockStyleSheet() {
    return QStringLiteral(R"QSS(
ads--CDockContainerWidget { background: #0e0e0e; }
ads--CDockContainerWidget > QSplitter { padding: 0; }
ads--CDockSplitter::handle { background: #0a0a0a; }
ads--CDockSplitter::handle:hover { background: #4e4e4e; }
ads--CDockAreaWidget { background: #1a1a1a; border: 0; }
ads--CDockAreaTitleBar { background: #1f1f1f; border-bottom: 1px solid #0c0c0c; min-height: 26px; max-height: 26px; }
ads--CDockAreaTabBar { background: #1f1f1f; }
ads--CDockWidgetTab { background: #1f1f1f; border: 0; border-right: 1px solid #141414; padding: 0 8px; }
ads--CDockWidgetTab QLabel { color: #8e8e8e; font-size: 11px; font-weight: 600; }
ads--CDockWidgetTab[activeTab="true"] { background: #262626; }
ads--CDockWidgetTab[activeTab="true"] QLabel { color: #e6e6e6; }
ads--CDockWidgetTab[focused="true"] { border-bottom: 2px solid #e6b450; }
ads--CDockWidget { background: #1a1a1a; border: 0; }
ads--CTitleBarButton { color: #8e8e8e; font-size: 10px; padding: 0 6px; }
ads--CTitleBarButton:disabled { color: #8e8e8e; }
ads--CFloatingDockContainer { background: #1a1a1a; }
)QSS");
}
