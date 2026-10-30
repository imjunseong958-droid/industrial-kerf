#pragma once

#include "editor_theme.h"

#include <QColor>
#include <QHash>
#include <QRegularExpression>
#include <QString>

namespace kerf {

// Replace each @token in `sheet` with the colour of the theme field it
// names, and @checkMark with the check box's tick. An unknown token is left
// in place, which the tests catch.
inline QString themedStyleSheet(QString sheet, const EditorTheme &t) {
    using Field = QColor EditorTheme::*;
    static const QHash<QString, Field> fields{
        {QStringLiteral("accent"), &EditorTheme::accent},
        {QStringLiteral("accentTop"), &EditorTheme::accentTop},
        {QStringLiteral("accentPressed"), &EditorTheme::accentPressed},
        {QStringLiteral("accentText"), &EditorTheme::accentText},
        {QStringLiteral("accentTint"), &EditorTheme::accentTint},
        {QStringLiteral("onAccent"), &EditorTheme::onAccent},
        {QStringLiteral("window"), &EditorTheme::window},
        {QStringLiteral("chrome"), &EditorTheme::chrome},
        {QStringLiteral("panel"), &EditorTheme::panel},
        {QStringLiteral("panelTitle"), &EditorTheme::panelTitle},
        {QStringLiteral("panelTitleActive"), &EditorTheme::panelTitleActive},
        {QStringLiteral("raised"), &EditorTheme::raised},
        {QStringLiteral("well"), &EditorTheme::well},
        {QStringLiteral("dialog"), &EditorTheme::dialog},
        {QStringLiteral("dockSeparator"), &EditorTheme::dockSeparator},
        {QStringLiteral("dockSeparatorHover"), &EditorTheme::dockSeparatorHover},
        {QStringLiteral("divider"), &EditorTheme::divider},
        {QStringLiteral("borderSubtle"), &EditorTheme::borderSubtle},
        {QStringLiteral("border"), &EditorTheme::border},
        {QStringLiteral("borderStrong"), &EditorTheme::borderStrong},
        {QStringLiteral("text"), &EditorTheme::text},
        {QStringLiteral("textStrong"), &EditorTheme::textStrong},
        {QStringLiteral("textSecondary"), &EditorTheme::textSecondary},
        {QStringLiteral("textTertiary"), &EditorTheme::textTertiary},
        {QStringLiteral("textDisabled"), &EditorTheme::textDisabled},
        {QStringLiteral("selection"), &EditorTheme::selection},
        {QStringLiteral("selectionText"), &EditorTheme::selectionText},
        {QStringLiteral("selectionInactive"), &EditorTheme::selectionInactive},
        {QStringLiteral("selectionInactiveText"), &EditorTheme::selectionInactiveText},
        {QStringLiteral("textSelection"), &EditorTheme::textSelection},
        {QStringLiteral("hover"), &EditorTheme::hover},
        {QStringLiteral("pressed"), &EditorTheme::pressed},
        {QStringLiteral("field"), &EditorTheme::field},
        {QStringLiteral("fieldBorder"), &EditorTheme::fieldBorder},
        {QStringLiteral("fieldDisabled"), &EditorTheme::fieldDisabled},
        {QStringLiteral("focusRing"), &EditorTheme::focusRing},
        {QStringLiteral("button"), &EditorTheme::button},
        {QStringLiteral("buttonEdge"), &EditorTheme::buttonEdge},
        {QStringLiteral("buttonTop"), &EditorTheme::buttonTop},
        {QStringLiteral("buttonBottom"), &EditorTheme::buttonBottom},
        {QStringLiteral("buttonHover"), &EditorTheme::buttonHover},
        {QStringLiteral("buttonPressed"), &EditorTheme::buttonPressed},
        {QStringLiteral("buttonText"), &EditorTheme::buttonText},
        {QStringLiteral("buttonDisabled"), &EditorTheme::buttonDisabled},
        {QStringLiteral("buttonDisabledEdge"), &EditorTheme::buttonDisabledEdge},
        {QStringLiteral("buttonDisabledText"), &EditorTheme::buttonDisabledText},
        {QStringLiteral("menu"), &EditorTheme::menu},
        {QStringLiteral("menuBorder"), &EditorTheme::menuBorder},
        {QStringLiteral("menuText"), &EditorTheme::menuText},
        {QStringLiteral("menuSelection"), &EditorTheme::menuSelection},
        {QStringLiteral("menuSelectionText"), &EditorTheme::menuSelectionText},
        {QStringLiteral("menuSeparator"), &EditorTheme::menuSeparator},
        {QStringLiteral("tooltip"), &EditorTheme::tooltip},
        {QStringLiteral("tooltipBorder"), &EditorTheme::tooltipBorder},
        {QStringLiteral("tooltipText"), &EditorTheme::tooltipText},
        {QStringLiteral("scrollTrack"), &EditorTheme::scrollTrack},
        {QStringLiteral("scrollHandle"), &EditorTheme::scrollHandle},
        {QStringLiteral("scrollHandleHover"), &EditorTheme::scrollHandleHover},
        {QStringLiteral("tabText"), &EditorTheme::tabText},
        {QStringLiteral("tabSelected"), &EditorTheme::tabSelected},
        {QStringLiteral("tabSelectedBorder"), &EditorTheme::tabSelectedBorder},
        {QStringLiteral("tabSelectedText"), &EditorTheme::tabSelectedText},
        {QStringLiteral("sliderGroove"), &EditorTheme::sliderGroove},
        {QStringLiteral("sliderGrooveBorder"), &EditorTheme::sliderGrooveBorder},
        {QStringLiteral("sliderHandle"), &EditorTheme::sliderHandle},
        {QStringLiteral("sliderHandleBorder"), &EditorTheme::sliderHandleBorder},
        {QStringLiteral("statusBar"), &EditorTheme::statusBar},
        {QStringLiteral("statusText"), &EditorTheme::statusText},
        {QStringLiteral("statusDivider"), &EditorTheme::statusDivider},
        {QStringLiteral("error"), &EditorTheme::error},
        {QStringLiteral("errorSurface"), &EditorTheme::errorSurface},
        {QStringLiteral("errorBorder"), &EditorTheme::errorBorder},
        {QStringLiteral("errorText"), &EditorTheme::errorText},
        {QStringLiteral("warning"), &EditorTheme::warning},
        {QStringLiteral("warningSurface"), &EditorTheme::warningSurface},
        {QStringLiteral("warningBorder"), &EditorTheme::warningBorder},
        {QStringLiteral("warningText"), &EditorTheme::warningText},
        {QStringLiteral("success"), &EditorTheme::success},
        {QStringLiteral("assistant"), &EditorTheme::assistant},
        {QStringLiteral("assistantStrong"), &EditorTheme::assistantStrong},
        {QStringLiteral("onAssistant"), &EditorTheme::onAssistant},
        {QStringLiteral("assistantSurface"), &EditorTheme::assistantSurface},
        {QStringLiteral("assistantBorder"), &EditorTheme::assistantBorder},
        {QStringLiteral("assistantText"), &EditorTheme::assistantText},
        {QStringLiteral("messageText"), &EditorTheme::messageText},
        {QStringLiteral("glyph"), &EditorTheme::glyph},
        {QStringLiteral("scrubTrack"), &EditorTheme::scrubTrack},
    };
    static const QRegularExpression token(QStringLiteral("@([A-Za-z][A-Za-z0-9]*)"));
    QString out;
    out.reserve(sheet.size());
    qsizetype last = 0;
    for (auto it = token.globalMatch(sheet); it.hasNext();) {
        const QRegularExpressionMatch match = it.next();
        QString value;
        if (match.captured(1) == QStringLiteral("checkMark")) {
            const QString image = checkMarkImagePath();
            value = image.isEmpty() ? QStringLiteral("none") : QStringLiteral("url(%1)").arg(image);
        } else {
            const auto field = fields.constFind(match.captured(1));
            if (field == fields.cend()) continue;
            const QColor color = t.*(field.value());
            value = color.alpha() == 255 ? color.name() : color.name(QColor::HexArgb);
        }
        out += QStringView(sheet).mid(last, match.capturedStart() - last);
        out += value;
        last = match.capturedEnd();
    }
    out += QStringView(sheet).mid(last);
    return out;
}

// The editor's application style sheet. Dark keeps the finishing-room
// surfaces (near-black wells, brightness steps instead of extra hues);
// light is macOS's light window family; both take the system accent for
// focus and the default action, and the system purple for agent work.
// Density follows desktop edit suites: 12px type, docked panels with title
// strips, and a status bar.
inline QString editorStyleSheet(const EditorTheme &theme) {
    return themedStyleSheet(QStringLiteral(R"QSS(
* {
    color: @text;
    font-size: 12px;
}

QWidget {
    background: transparent;
}
QMainWindow,
QWidget#workspaceRoot {
    background: @window;
}
QLabel {
    background: transparent;
}
QDialog { background: @dialog; }
QToolTip {
    color: @tooltipText;
    background: @tooltip;
    border: 1px solid @tooltipBorder;
    padding: 5px 8px;
}
QComboBox QAbstractItemView {
    background: @menu;
    color: @menuText;
    border: 1px solid @menuBorder;
    selection-background-color: @menuSelection;
    selection-color: @menuSelectionText;
}

QMenuBar {
    background: @chrome;
    border-bottom: 1px solid @divider;
    padding: 2px 6px;
}
QMenuBar::item {
    padding: 3px 8px;
    border-radius: 3px;
}
QMenuBar::item:selected { background: @pressed; }
QMenu {
    background: @menu;
    border: 1px solid @menuBorder;
    padding: 3px;
}
QMenu::item {
    padding: 4px 24px 4px 10px;
    border-radius: 3px;
}
QMenu::separator { height: 1px; background: @menuSeparator; margin: 3px 6px; }
QMenu::item:selected { background: @menuSelection; color: @menuSelectionText; }
QMenu::item:disabled { color: @textDisabled; background: transparent; }

QToolBar {
    background: @chrome;
    border: 0;
    border-bottom: 1px solid @divider;
    spacing: 2px;
    padding: 0px 6px;
}
QToolBar::separator {
    background: @border;
    width: 1px;
    margin: 2px 3px;
}

QWidget#panel,
QWidget#assistantPanel,
QFrame#viewerFrame,
QWidget#timelinePanel {
    background: @panel;
    border: 0;
    border-radius: 0px;
}
QFrame#timelineFrame {
    background: transparent;
    border: 0;
}
QWidget#transport {
    background: @panel;
    border: 0;
    border-top: 1px solid @divider;
    border-radius: 0px;
}
QLabel#transportTime { color: @accentText; font-family: Menlo, Monaco, monospace; font-size: 13px; font-weight: 600; }
QLabel#transportDuration { color: @textTertiary; font-family: Menlo, Monaco, monospace; font-size: 11px; }
QSplitter::handle { background: @dockSeparator; }
QSplitter::handle:hover { background: @dockSeparatorHover; }
QSplitter::handle:horizontal { width: 2px; }
QSplitter::handle:vertical { height: 2px; }

QToolButton,
QPushButton {
    color: @buttonText;
    background: @button;
    border: 1px solid @buttonEdge;
    border-top-color: @buttonTop;
    border-bottom-color: @buttonBottom;
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
    background: @buttonHover;
    border-color: @borderStrong;
}
QToolBar QToolButton:pressed,
QToolButton:pressed,
QPushButton:pressed {
    background: @buttonPressed;
    border-color: @borderStrong;
}
QToolButton:focus,
QPushButton:focus {
    border-color: @focusRing;
}
QToolBar QToolButton:disabled,
QToolButton:disabled,
QPushButton:disabled {
    color: @buttonDisabledText;
    background: @buttonDisabled;
    border-color: @buttonDisabledEdge;
}
/* A toolbar's unavailable commands are dimmed, not boxed, as in macOS. */
QToolBar QToolButton:disabled { background: transparent; border-color: transparent; }
QPushButton#primaryButton,
QPushButton#chatSendButton {
    background: qlineargradient(x1: 0, y1: 0, x2: 0, y2: 1, stop: 0 @accentTop, stop: 1 @accent);
    border-color: @accent;
    border-top-color: @accentTop;
    color: @onAccent;
    font-weight: 600;
}
QPushButton#primaryButton:hover,
QPushButton#chatSendButton:hover { background: @accentTop; }
QPushButton#primaryButton:pressed,
QPushButton#chatSendButton:pressed { background: @accentPressed; border-color: @accentPressed; }
QPushButton#primaryButton:disabled,
QPushButton#chatSendButton:disabled {
    color: @buttonDisabledText;
    background: @buttonDisabled;
    border-color: @buttonDisabledEdge;
}
QPushButton#dangerButton { background: @button; border-color: @borderStrong; }
QPushButton#dangerButton:hover { background: @buttonHover; }
QPushButton#secondaryButton { background: @raised; }
QToolButton#squareButton {
    background: transparent;
    border-color: transparent;
    padding: 2px 9px;
    font-size: 18px;
    color: @glyph;
}
QToolButton#squareButton:hover { background: @hover; border-color: @border; }
QToolButton#playButton {
    background: @button;
    border-color: @buttonEdge;
    border-top-color: @buttonTop;
    border-bottom-color: @buttonBottom;
    border-radius: 4px;
    color: @textStrong;
    font-size: 15px;
    padding: 4px 14px;
}
QToolButton#playButton:hover { background: @buttonHover; }
QToolButton#playButton:disabled { background: @buttonDisabled; color: @buttonDisabledText; border-color: @buttonDisabledEdge; }
QToolButton#transportButton,
QPushButton#transportButton {
    background: transparent;
    border: 1px solid transparent;
    border-radius: 3px;
    color: @text;
    padding: 1px 5px;
    min-height: 18px;
    min-width: 18px;
    font-size: 11px;
}
QToolButton#transportButton:disabled { background: transparent; border-color: transparent; }
QToolButton#transportButton:hover,
QPushButton#transportButton:hover {
    background: @hover;
    border-color: @border;
    color: @textStrong;
}
QToolBar QComboBox#qualityPicker,
QToolBar QComboBox#canvasPicker {
    background: @button;
    border: 1px solid @buttonEdge;
    border-top-color: @buttonTop;
    border-bottom-color: @buttonBottom;
    border-radius: 4px;
    padding: 0px 18px 0px 6px;
    min-height: 18px;
}
QToolBar QComboBox#qualityPicker:hover,
QToolBar QComboBox#canvasPicker:hover {
    background: @buttonHover;
    border-color: @borderStrong;
}
QToolBar QComboBox#qualityPicker::drop-down,
QToolBar QComboBox#canvasPicker::drop-down {
    border: 0;
    width: 18px;
}
QToolBar QToolButton#toolbarPrimary {
    background: qlineargradient(x1: 0, y1: 0, x2: 0, y2: 1, stop: 0 @accentTop, stop: 1 @accent);
    border: 1px solid @accent;
    border-radius: 4px;
    color: @onAccent;
    font-weight: 600;
    padding: 0px 8px;
}
QToolBar QToolButton#toolbarPrimary:hover { background: @accentTop; }
QToolBar QToolButton#toolbarPrimary:disabled {
    color: @buttonDisabledText;
    background: @buttonDisabled;
    border-color: @buttonDisabledEdge;
}
QToolBar QToolButton#toolbarQuiet {
    background: transparent;
    border: 1px solid transparent;
    color: @text;
    padding: 0px 6px;
}
QToolBar QToolButton#toolbarQuiet:hover {
    background: @hover;
    border-color: @border;
    color: @textStrong;
}
QToolBar QToolButton#toolbarQuiet:disabled {
    color: @textDisabled;
    background: transparent;
    border-color: transparent;
}
QToolButton#mediaToggle {
    background: transparent;
    border: 0;
    color: @textSecondary;
    padding: 1px 5px;
}
QToolButton#mediaToggle:hover { color: @text; background: @hover; }
QToolButton#abortButton {
    color: @text;
    background: transparent;
    border-color: transparent;
    padding: 5px 7px;
}
QToolButton#abortButton:hover { background: @hover; border-color: @borderStrong; }
QToolButton#abortButton:disabled { color: @textDisabled; background: transparent; border-color: transparent; }

QLabel#eyebrow { color: @textSecondary; font-size: 12px; font-weight: 500; letter-spacing: 0px; }
QLabel#muted { color: @textSecondary; font-size: 11px; }
QLabel#chatHint { color: @textSecondary; font-size: 11px; padding: 6px 2px 2px; }
QLabel#statusPill {
    color: @textSecondary;
    background: @raised;
    border: 1px solid @border;
    border-radius: 4px;
    padding: 2px 6px;
    font-size: 10px;
}
QLabel#timecode { color: @text; font-family: Menlo, Monaco, monospace; font-size: 11px; }
QLabel#inspectorClip { color: @textStrong; font-weight: 600; padding: 5px 0; }
QLabel#projectTitle { color: @text; font-weight: 600; padding: 0 20px; }
QWidget#assistantPanel { background: @well; border-left: 1px solid @divider; }
QWidget#assistantPanel QLabel#statusPill {
    color: @assistantText;
    background: @assistantSurface;
    border-color: @assistantBorder;
    border-radius: 3px;
    padding: 0px 5px;
    font-size: 9px;
}
QLabel#composerStatus { color: @textSecondary; font-size: 10px; }
QLabel#chatActivity {
    color: @assistantText;
    background: @assistantSurface;
    border-left: 2px solid @assistant;
    padding: 0 8px;
    font-size: 11px;
}
QLabel#proposalSummary { color: @text; }
QToolButton#eyebrow { text-align: left; background: transparent; border: 0; color: @textSecondary; }

QListWidget,
QTextEdit,
QPlainTextEdit,
QLineEdit,
QDoubleSpinBox,
QComboBox {
    background: @field;
    border: 1px solid @fieldBorder;
    border-radius: 3px;
    padding: 4px;
    selection-background-color: @textSelection;
    selection-color: @text;
}
QListWidget:hover,
QTextEdit:hover,
QPlainTextEdit:hover,
QLineEdit:hover,
QDoubleSpinBox:hover,
QComboBox:hover { border-color: @borderStrong; }
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
    border-left: 1px solid @fieldBorder;
    border-top-right-radius: 4px;
    border-bottom-right-radius: 4px;
}
QListWidget:focus,
QTextEdit:focus,
QPlainTextEdit:focus,
QLineEdit:focus,
QDoubleSpinBox:focus,
QComboBox:focus {
    border-color: @focusRing;
    background: @field;
}
/* No dotted focus frame on the current item: the selection shows it, as in macOS. */
QListWidget { outline: 0; }
QListWidget::item { padding: 4px; border: 0; border-radius: 4px; }
QListWidget::item:hover { background: @hover; }
QListWidget::item:selected { background: @selectionInactive; color: @selectionInactiveText; }
QListWidget::item:selected:focus,
QListWidget#projectBin::item:selected:focus,
QListWidget#presetList::item:selected:focus { background: @selection; color: @selectionText; }
QListWidget#mediaList,
QListWidget#activityList,
QScrollArea#mediaList { background: transparent; border: 0; padding: 0; }
QWidget#mediaGrid { background: transparent; }
QTextEdit#conversation { background: transparent; border: 0; padding: 4px 0; color: @messageText; }
QTextEdit#chatInput {
    background: transparent;
    border: 0;
    padding: 0;
    color: @text;
    font-size: 13px;
}
QTextEdit#chatInput:focus { background: transparent; border: 0; }
QListWidget:disabled,
QTextEdit:disabled,
QPlainTextEdit:disabled,
QLineEdit:disabled,
QDoubleSpinBox:disabled,
QComboBox:disabled {
    color: @textDisabled;
    background: @fieldDisabled;
    border-color: @borderSubtle;
}
QComboBox#agentModel {
    background: transparent;
    border: 0;
    padding: 3px 2px;
    font-size: 11px;
    color: @textSecondary;
}
QComboBox#agentModel:hover { background: @hover; }
QComboBox#agentModel QLineEdit { background: transparent; border: 0; padding: 0; }
QFrame#chatComposer {
    background: @field;
    border: 1px solid @fieldBorder;
    border-radius: 12px;
}
QFrame#chatComposer[state="focused"] { border-color: @focusRing; background: @field; }
QFrame#chatComposer[state="sending"] { border-color: @assistantBorder; background: @assistantSurface; }
QFrame#chatComposer[state="offline"] { border-color: @borderSubtle; background: @fieldDisabled; }
QTextEdit#chatInput:disabled { background: transparent; color: @textDisabled; border: 0; }
QPushButton#chatSendButton { border-radius: 8px; padding: 0; font-size: 19px; }
QToolButton#abortButton { border: 1px solid @assistantBorder; border-radius: 7px; padding: 4px 9px; }

QGroupBox#proposalBox,
QGroupBox#providerBox,
QGroupBox#inboundMcpSection,
QGroupBox#mcpSection {
    border: 1px solid @border;
    border-radius: 3px;
    margin-top: 10px;
    padding: 10px 8px 8px;
    background: @raised;
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
    color: @textSecondary;
    font-size: 11px;
    font-weight: 600;
}
QProgressBar { background: @borderSubtle; border: 0; border-radius: 2px; height: 4px; max-height: 4px; }
QProgressBar::chunk { background: @accent; border-radius: 3px; }

QTabWidget::pane { border: 0; background: transparent; }
QTabBar { background: transparent; }
QTabBar::tab {
    color: @tabText;
    background: transparent;
    padding: 4px 10px;
    border: 1px solid transparent;
    border-radius: 4px;
}
QTabBar::tab:hover { color: @text; background: @hover; }
QTabBar::tab:selected {
    color: @tabSelectedText;
    background: @tabSelected;
    border-color: @tabSelectedBorder;
}

QSlider::groove:horizontal { background: @sliderGroove; border: 1px solid @sliderGrooveBorder; height: 5px; border-radius: 3px; }
QSlider::handle:horizontal { background: @sliderHandle; border: 1px solid @sliderHandleBorder; width: 11px; margin: -4px 0; border-radius: 4px; }
QScrollBar:vertical, QScrollBar:horizontal { background: @scrollTrack; border: 0; }
QScrollBar:vertical { width: 8px; }
QScrollBar:horizontal { height: 8px; }
QScrollBar::handle:vertical, QScrollBar::handle:horizontal { background: @scrollHandle; border-radius: 4px; min-height: 20px; min-width: 20px; }
QScrollBar::handle:vertical:hover, QScrollBar::handle:horizontal:hover { background: @scrollHandleHover; }
QScrollBar::add-line, QScrollBar::sub-line { width: 0; height: 0; }
QStatusBar { background: @statusBar; color: @statusText; border-top: 1px solid @divider; font-size: 10px; min-height: 20px; max-height: 20px; }
QStatusBar::item { border: 0; }
QStatusBar QLabel { color: @statusText; font-size: 10px; padding: 0 8px; border-left: 1px solid @statusDivider; }
QStatusBar QLabel#statusConnection[state="ready"] { color: @success; }
QStatusBar QLabel#statusConnection[state="connecting"] { color: @warning; }
QStatusBar QLabel#statusConnection[state="disconnected"] { color: @error; }
QFrame#connectionBanner,
QFrame#errorBanner { background: @errorSurface; border: 1px solid @errorBorder; border-radius: 4px; }
QToolBar#toolPalette { background: @chrome; border: 0; border-right: 1px solid @divider; spacing: 2px; padding: 4px 2px; }
QToolBar#toolPalette QToolButton { color: @textSecondary; font-size: 15px; min-width: 24px; min-height: 24px; border-radius: 3px; }
QToolBar#toolPalette QToolButton:hover { background: @hover; color: @textStrong; }
QToolBar#toolPalette QToolButton:checked { background: @accent; color: @onAccent; }
QListWidget#projectBin { background: @well; border: 0; }
QListWidget#projectBin::item { color: @text; border-radius: 3px; padding: 4px; }
QListWidget#projectBin::item:selected { background: @selectionInactive; color: @selectionInactiveText; }
QWidget#sourceMonitor { background: @panel; }
QFrame#askPrompt { background: @raised; border: 1px solid @assistant; border-radius: 8px; }
QLabel#askTitle { color: @assistantText; font-size: 12px; font-weight: 600; }
QTextEdit#askInput { background: @field; border: 1px solid @fieldBorder; border-radius: 5px; padding: 4px; color: @text; }
QFrame#aiBar { background: @assistantSurface; border: 0; border-bottom: 1px solid @assistantBorder; }
QLabel#aiBarText { color: @assistantText; font-size: 11px; }
QFrame#aiBar QToolButton { color: @text; padding: 2px 10px; border: 1px solid @assistantBorder; border-radius: 4px; }
QFrame#aiBar QToolButton:hover { background: @buttonHover; }
QFrame#aiBar QToolButton#aiAccept { background: @assistantStrong; color: @onAssistant; border-color: @assistantStrong; font-weight: 600; }
QPushButton#nextStep { color: @assistantText; background: @assistantSurface; padding: 3px 10px; border: 1px solid @assistantBorder; border-radius: 11px; font-size: 11px; }
QPushButton#nextStep:hover { background: @assistantSurface; border-color: @assistant; }
QFrame#aiBar QToolButton:disabled { color: @textDisabled; background: transparent; border-color: @borderSubtle; }

QFrame#divergedBanner { background: @warningSurface; border: 1px solid @warningBorder; border-radius: 4px; }
QLabel#divergedMessage { color: @warningText; font-size: 11px; }
QPushButton#loadFromDisk { background: @accent; border-color: @accent; color: @onAccent; font-weight: 600; }
QPushButton#loadFromDisk:hover { background: @accentTop; }
QStatusBar QToolButton#syncChip {
    background: transparent;
    border: 0;
    border-left: 1px solid @statusDivider;
    border-radius: 0;
    color: @statusText;
    font-size: 10px;
    padding: 0 8px;
    min-height: 0;
}
QStatusBar QToolButton#syncChip:hover { color: @textStrong; background: @hover; }
QStatusBar QToolButton#syncChip[state="local"] { color: @textTertiary; }
QStatusBar QToolButton#syncChip[state="syncing"] { color: @accentText; }
QStatusBar QToolButton#syncChip[state="live"] { color: @success; }
QStatusBar QToolButton#syncChip[state="conflict"],
QStatusBar QToolButton#syncChip[state="failed"] { color: @error; }
QWidget#presetsPanel { background: @panel; }
QLabel#presetsTitle { color: @textSecondary; font-size: 10px; font-weight: 600; }
QListWidget#presetList { background: @well; border: 0; padding: 2px; }
QListWidget#presetList::item { padding: 6px; border-radius: 4px; }
QListWidget#presetList::item:selected { background: @selectionInactive; color: @selectionInactiveText; }
QToolButton#presetsRefresh { background: transparent; border: 0; padding: 0 5px; }

QCheckBox::indicator,
QRadioButton::indicator {
    width: 12px;
    height: 12px;
    background: @field;
    border: 1px solid @borderStrong;
}
QCheckBox::indicator { border-radius: 3px; }
QRadioButton::indicator { border-radius: 7px; }
QCheckBox::indicator:checked { background: @accent; border-color: @accent; image: @checkMark; }
QRadioButton::indicator:checked {
    background: qradialgradient(cx: 0.5, cy: 0.5, radius: 0.5, fx: 0.5, fy: 0.5,
                                stop: 0 @onAccent, stop: 0.3 @onAccent, stop: 0.42 @accent, stop: 1 @accent);
    border-color: @accent;
}
QCheckBox::indicator:disabled,
QRadioButton::indicator:disabled { background: @fieldDisabled; border-color: @borderSubtle; }
QCheckBox::indicator:checked:disabled { background: @textDisabled; border-color: @textDisabled; }
QRadioButton::indicator:checked:disabled {
    background: qradialgradient(cx: 0.5, cy: 0.5, radius: 0.5, fx: 0.5, fy: 0.5,
                                stop: 0 @fieldDisabled, stop: 0.3 @fieldDisabled, stop: 0.42 @textDisabled, stop: 1 @textDisabled);
    border-color: @textDisabled;
}

QDialog#publishDialog QCheckBox { spacing: 7px; }
QDialog#publishDialog QCheckBox::indicator {
    width: 12px;
    height: 12px;
    background: @field;
    border: 1px solid @borderStrong;
    border-radius: 3px;
}
QDialog#publishDialog QCheckBox::indicator:checked { background: @accent; border-color: @accent; }
QDialog#publishDialog QCheckBox::indicator:disabled { background: @fieldDisabled; border-color: @borderSubtle; }
)QSS"), theme);
}

// Docked panels, layered over the dock manager's own style sheet: title
// strips with the panel names as tabs and an accent line under the focused
// one.
//
// The dock manager's sheet draws a black close button, and a white one on
// the focused tab (which it fills with the highlight); here no tab is filled,
// so every tab takes the icons for the appearance, from the dock manager's
// own resources.
inline QString dockStyleSheet(const EditorTheme &theme) {
    const QString icons = theme.dark ? QStringLiteral("_dark") : QString();
    return themedStyleSheet(QStringLiteral(R"QSS(
ads--CDockContainerWidget { background: @window; }
ads--CDockContainerWidget > QSplitter { padding: 0; }
ads--CDockSplitter::handle { background: @dockSeparator; }
ads--CDockSplitter::handle:hover { background: @dockSeparatorHover; }
ads--CDockAreaWidget { background: @panel; border: 0; }
ads--CDockAreaTitleBar { background: @panelTitle; border-bottom: 1px solid @divider; min-height: 26px; max-height: 26px; }
ads--CDockAreaTabBar { background: @panelTitle; }
ads--CDockWidgetTab { background: @panelTitle; border: 0; border-right: 1px solid @divider; padding: 0 8px; }
ads--CDockWidgetTab QLabel { color: @textSecondary; font-size: 11px; font-weight: 600; }
ads--CDockWidgetTab[activeTab="true"] { background: @panelTitleActive; }
ads--CDockWidgetTab[activeTab="true"] QLabel { color: @text; }
ads--CDockWidgetTab[focused="true"] { border-right: 1px solid @divider; border-bottom: 2px solid @accent; }
ads--CDockWidgetTab > #tabCloseButton,
ads--CDockWidgetTab[focused="true"] > #tabCloseButton {
    qproperty-icon: url(:/ads/images/close-button$icons.svg), url(:/ads/images/close-button-disabled$icons.svg) disabled;
    border-radius: 3px;
}
ads--CDockWidgetTab > #tabCloseButton:hover,
ads--CDockWidgetTab[focused="true"] > #tabCloseButton:hover { background: @hover; }
ads--CDockWidgetTab > #tabCloseButton:pressed,
ads--CDockWidgetTab[focused="true"] > #tabCloseButton:pressed { background: @pressed; }
#tabsMenuButton { qproperty-icon: url(:/ads/images/tabs-menu-button$icons.svg); }
ads--CDockWidget { background: @panel; border: 0; }
ads--CTitleBarButton { color: @textSecondary; font-size: 10px; padding: 0 6px; }
ads--CTitleBarButton:disabled { color: @textSecondary; }
ads--CFloatingDockContainer { background: @panel; }
)QSS").replace(QStringLiteral("$icons"), icons), theme);
}

} // namespace kerf
