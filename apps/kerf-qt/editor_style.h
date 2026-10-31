#pragma once

#include "editor_common.h"
#include "editor_theme.h"

#include <QColor>
#include <QHash>
#include <QRegularExpression>
#include <QString>
#include <utility>

namespace kerf {

// Replace each @token in `sheet` with the colour of the theme field it
// names, @checkMark with the check box's tick, @popUpChevrons (and its
// Disabled twin) with a pop-up button's chevrons, and the other images
// below with their symbols. (Steppers draw their own: see StepperSpinBox.)
// An unknown token is left in place, which the tests catch.
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
        {QStringLiteral("group"), &EditorTheme::group},
        {QStringLiteral("groupBorder"), &EditorTheme::groupBorder},
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
        {QStringLiteral("tabInactiveText"), &EditorTheme::tabInactiveText},
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
        {QStringLiteral("placeholderText"), &EditorTheme::placeholderText},
        {QStringLiteral("checkFill"), &EditorTheme::checkFill},
        {QStringLiteral("checkBorder"), &EditorTheme::checkBorder},
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
        {QStringLiteral("warningTitle"), &EditorTheme::warningTitle},
        {QStringLiteral("success"), &EditorTheme::success},
        {QStringLiteral("assistant"), &EditorTheme::assistant},
        {QStringLiteral("assistantSurface"), &EditorTheme::assistantSurface},
        {QStringLiteral("assistantBorder"), &EditorTheme::assistantBorder},
        {QStringLiteral("assistantText"), &EditorTheme::assistantText},
        {QStringLiteral("assistantHover"), &EditorTheme::assistantHover},
        {QStringLiteral("messageText"), &EditorTheme::messageText},
        {QStringLiteral("glyph"), &EditorTheme::glyph},
        {QStringLiteral("toolSelected"), &EditorTheme::toolSelected},
        {QStringLiteral("scrubTrack"), &EditorTheme::scrubTrack},
    };
    static const QRegularExpression token(QStringLiteral("@([A-Za-z][A-Za-z0-9]*)"));
    QString out;
    out.reserve(sheet.size());
    qsizetype last = 0;
    for (auto it = token.globalMatch(sheet); it.hasNext();) {
        const QRegularExpressionMatch match = it.next();
        // Images: the tick, pop-up chevrons, menu checks and submenu
        // chevrons in the text colours, and the dock tabs' symbols in the
        // glyph colours.
        using ImagePath = QString (*)(const QColor &);
        static const QHash<QString, std::pair<ImagePath, QColor EditorTheme::*>> images{
            {QStringLiteral("popUpChevrons"), {popUpChevronsImagePath, &EditorTheme::text}},
            {QStringLiteral("popUpChevronsDisabled"), {popUpChevronsImagePath, &EditorTheme::textDisabled}},
            {QStringLiteral("menuCheck"), {menuCheckImagePath, &EditorTheme::menuText}},
            {QStringLiteral("menuCheckSelected"), {menuCheckImagePath, &EditorTheme::menuSelectionText}},
            {QStringLiteral("menuCheckDisabled"), {menuCheckImagePath, &EditorTheme::textDisabled}},
            {QStringLiteral("menuChevron"), {menuChevronImagePath, &EditorTheme::menuText}},
            {QStringLiteral("menuChevronSelected"), {menuChevronImagePath, &EditorTheme::menuSelectionText}},
            {QStringLiteral("menuChevronDisabled"), {menuChevronImagePath, &EditorTheme::textDisabled}},
            {QStringLiteral("closeMark"), {closeMarkImagePath, &EditorTheme::glyph}},
            {QStringLiteral("closeMarkDisabled"), {closeMarkImagePath, &EditorTheme::glyphDisabled}},
            {QStringLiteral("tabsMenuMark"), {tabsMenuImagePath, &EditorTheme::glyph}},
        };
        const auto image = [](const QString &path) {
            return path.isEmpty() ? QStringLiteral("none") : QStringLiteral("url(%1)").arg(path);
        };
        QString value;
        const QString name = match.captured(1);
        if (name == QStringLiteral("checkMark")) {
            value = image(checkMarkImagePath());
        } else if (const auto drawn = images.constFind(name); drawn != images.cend()) {
            value = image(drawn->first(t.*(drawn->second)));
        } else {
            const auto field = fields.constFind(name);
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

// The editor's application style sheet: macOS's light and dark window
// families, as the project launcher draws them, with the system accent for
// focus and the default action and the system purple marking agent work.
// Density follows desktop edit suites: 12px type, docked panels with title
// strips, and a status bar.
inline QString editorStyleSheet(const EditorTheme &theme) {
    return themedStyleSheet(QStringLiteral(R"QSS(
* {
    color: @text;
    /* Placeholders carry guidance, so they read as text. Without this, Qt
       draws them in each widget's text colour at half strength. */
    placeholder-text-color: @placeholderText;
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
/* A pop-up button's menu: rounded as macOS's, its rows drawn by
   PopupItemDelegate (a check by the current choice, the accent under the
   pointer). Its window is translucent, so the corners are clipped. */
QComboBox QAbstractItemView {
    background: @menu;
    color: @menuText;
    border: 1px solid @menuBorder;
    border-radius: 6px;
    padding: 4px;
    outline: 0;
}
QComboBox QAbstractItemView::item { min-height: 22px; padding: 0; border: 0; }

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
/* Menus Qt draws (context menus, and every menu off macOS) take macOS's
   shape: 6px corners, clipped by a translucent window, and rounded rows
   with room for a check. They show no symbols (see useEditorStyle). */
QMenu {
    background: @menu;
    border: 1px solid @menuBorder;
    border-radius: 6px;
    padding: 4px;
    /* Qt still sets a menu's text past its commands' symbols, by their
       size and 4px more; this leaves the 4px. */
    icon-size: 0px;
}
QMenu::item {
    padding: 3px 18px 3px 22px;
    border-radius: 4px;
}
/* Qt starts a row's text past the check column, in a menu with checks, or
   past the 4px above, in one whose commands have symbols; such menus pad
   their rows by less (useEditorStyle marks them), so text starts 22px in
   either way. A check sits where a pop-up's rows draw theirs. */
QMenu[iconColumn="true"]::item { padding-left: 18px; }
QMenu[checkColumn="true"]::item { padding-left: 4px; }
QMenu::indicator { width: 18px; height: 14px; }
QMenu::indicator:checked { image: @menuCheck; }
QMenu::indicator:checked:selected { image: @menuCheckSelected; }
QMenu::indicator:checked:disabled { image: @menuCheckDisabled; }
/* A submenu's chevron, thin as macOS's, 8px in from the row's end. */
QMenu::right-arrow { image: @menuChevron; width: 6px; height: 10px; right: 8px; }
QMenu::right-arrow:selected { image: @menuChevronSelected; }
QMenu::right-arrow:disabled { image: @menuChevronDisabled; }
QMenu::separator { height: 1px; background: @menuSeparator; margin: 4px 6px; }
QMenu::item:selected { background: @menuSelection; color: @menuSelectionText; }
QMenu::item:disabled { color: @textDisabled; background: transparent; }

/* The toolbar groups its commands with space, not separators, as macOS
   toolbars do. */
QToolBar {
    background: @chrome;
    border: 0;
    border-bottom: 1px solid @divider;
    spacing: 2px;
    padding: 0px 6px;
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
QLabel#transportDuration { color: @textSecondary; font-family: Menlo, Monaco, monospace; font-size: 11px; }
QSplitter::handle { background: @dockSeparator; }
QSplitter::handle:hover { background: @dockSeparatorHover; }
QSplitter::handle:horizontal { width: 2px; }
QSplitter::handle:vertical { height: 2px; }

/* Push buttons are 22px with a 5px radius, as the launcher's and macOS's
   regular controls. They take focus from Tab alone, and show it as a soft
   ring outside the bezel (see useEditorStyle), not Fusion's inner frame. */
QToolButton,
QPushButton {
    color: @buttonText;
    background: @button;
    border: 1px solid @buttonEdge;
    border-top-color: @buttonTop;
    border-bottom-color: @buttonBottom;
    border-radius: 5px;
    padding: 2px 12px;
    min-height: 16px;
    max-height: 16px;
    outline: 0;
}
/* Symbol-only buttons keep a narrow bezel. */
QToolButton#maskTool,
QToolButton#maskRemove,
QToolButton#effectRemove { padding: 2px 5px; }
/* A colour well: its swatch fills a small bezel. */
QPushButton[colorWell="true"] { padding: 2px 0; }
QToolBar QToolButton {
    background: transparent;
    border-color: transparent;
    min-width: 20px;
    min-height: 16px;
    max-height: 16px;
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
/* A dialog's default button is filled with the accent, as macOS draws it. */
QDialogButtonBox QPushButton:default {
    background: qlineargradient(x1: 0, y1: 0, x2: 0, y2: 1, stop: 0 @accentTop, stop: 1 @accent);
    border-color: @accent;
    border-top-color: @accentTop;
    color: @onAccent;
}
QDialogButtonBox QPushButton:default:hover { background: @accentTop; }
QDialogButtonBox QPushButton:default:pressed { background: @accentPressed; border-color: @accentPressed; }
QDialogButtonBox QPushButton:default:disabled {
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
    border-radius: 4px;
    color: @text;
    padding: 1px 5px;
    min-height: 18px;
    max-height: 18px;
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
    border-radius: 5px;
    padding: 0px 22px 0px 8px;
    min-height: 18px;
    max-height: 18px;
}
QToolBar QComboBox#qualityPicker:hover,
QToolBar QComboBox#canvasPicker:hover { background: @buttonHover; }
QToolBar QToolButton#toolbarPrimary {
    background: qlineargradient(x1: 0, y1: 0, x2: 0, y2: 1, stop: 0 @accentTop, stop: 1 @accent);
    border: 1px solid @accent;
    border-radius: 5px;
    color: @onAccent;
    font-weight: 600;
    padding: 0px 8px;
    /* As tall as the pop-ups beside it. */
    min-height: 18px;
    max-height: 18px;
}
QToolBar QToolButton#toolbarPrimary:hover { background: @accentTop; }
QToolBar QToolButton#toolbarPrimary:disabled {
    color: @buttonDisabledText;
    background: @buttonDisabled;
    border-color: @buttonDisabledEdge;
}
/* Render: a push button's bezel, as tall as the pop-ups beside it. */
QToolBar QToolButton#toolbarQuiet {
    background: @button;
    border: 1px solid @buttonEdge;
    border-top-color: @buttonTop;
    border-bottom-color: @buttonBottom;
    border-radius: 5px;
    color: @buttonText;
    padding: 0px 8px;
    min-height: 18px;
    max-height: 18px;
}
QToolBar QToolButton#toolbarQuiet:hover { background: @buttonHover; }
QToolBar QToolButton#toolbarQuiet:pressed { background: @buttonPressed; }
QToolBar QToolButton#toolbarQuiet:disabled {
    color: @buttonDisabledText;
    background: @buttonDisabled;
    border-color: @buttonDisabledEdge;
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
    padding: 1px 6px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0px;
}
QLabel#timecode { color: @text; font-family: Menlo, Monaco, monospace; font-size: 11px; }
QLabel#inspectorClip { color: @textStrong; font-weight: 600; padding: 5px 0 5px 0; margin: 0; }
/* Each section, effect and mask is headed by its name in semibold, as the
   clip's is; a hairline sets an effect or mask apart from the one above. */
QCheckBox#effectEnabled, QLabel#inspectorSection, QToolButton#maskName { font-weight: 600; }
QWidget#effectBox[following="true"], QWidget#maskBox[following="true"] { border-top: 1px solid @divider; }
/* The window's title, 13px semibold as macOS's and the launcher's. */
QLabel#projectTitle { color: @text; font-size: 13px; font-weight: 600; padding: 0; }
QWidget#assistantPanel { background: @well; border-left: 1px solid @divider; }
QFrame#scrollEdge { background: transparent; border: 0; }
QFrame#scrollEdge[scrolled="true"] { background: @divider; }
QWidget#assistantPanel QLabel#statusPill {
    color: @assistantText;
    background: @assistantSurface;
    border-color: @assistantBorder;
    border-radius: 4px;
    padding: 1px 6px;
    font-size: 11px;
    font-weight: 600;
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
QToolButton#eyebrow { text-align: left; background: transparent; border: 0; color: @textSecondary; padding: 2px 0; }
QToolButton#eyebrow:hover { color: @text; background: transparent; }
/* A disclosure: a chevron and a label, no bezel, as macOS's Details rows. */
QToolButton#disclosure {
    background: transparent;
    border: 0;
    border-radius: 0;
    padding: 2px 0;
    min-height: 0;
    color: @text;
    font-size: 13px;
}
QToolButton#disclosure:hover,
QToolButton#disclosure:pressed { background: transparent; border: 0; }

QListWidget,
QTextEdit,
QPlainTextEdit {
    background: @field;
    border: 1px solid @fieldBorder;
    border-radius: 3px;
    padding: 4px;
    selection-background-color: @textSelection;
    selection-color: @text;
}
/* Single-line fields and pop-ups are 22px with a 5px radius, as the
   launcher's (a pop-up takes a button's bezel below). */
QLineEdit,
QDoubleSpinBox,
QComboBox {
    background: @field;
    border: 1px solid @fieldBorder;
    border-radius: 5px;
    padding: 2px 7px;
    min-height: 16px;
    max-height: 16px;
    selection-background-color: @textSelection;
    selection-color: @text;
}
QListWidget:hover,
QTextEdit:hover,
QPlainTextEdit:hover,
QLineEdit:hover,
QDoubleSpinBox:hover,
QComboBox:hover { border-color: @borderStrong; }
/* Pop-up buttons and steppers: macOS's chevrons on the field, with no
   bevelled arrow cells. A pop-up button's height is its content's, with no
   padding above or below: Qt reads that padding as its menu's margin. */
QComboBox { padding: 0 24px 0 8px; min-height: 20px; max-height: 20px; combobox-popup: 1; }
QComboBox::drop-down {
    subcontrol-origin: padding;
    subcontrol-position: center right;
    width: 22px;
    border: 0;
    background: transparent;
}
QComboBox::down-arrow { image: @popUpChevrons; width: 10px; height: 14px; border: 0; }
QComboBox::down-arrow:disabled { image: @popUpChevronsDisabled; }
/* A pop-up button is a push button's bezel with chevrons, as macOS draws
   one and the launcher its Frame Rate; a combo box that takes typing (the
   Assistant's Model) stays a field. */
QComboBox:!editable {
    color: @buttonText;
    background: @button;
    border-color: @buttonEdge;
    border-top-color: @buttonTop;
    border-bottom-color: @buttonBottom;
}
QComboBox:!editable:hover {
    background: @buttonHover;
    border-color: @buttonEdge;
    border-top-color: @buttonTop;
    border-bottom-color: @buttonBottom;
}
QComboBox:!editable:disabled {
    color: @buttonDisabledText;
    background: @buttonDisabled;
    border-color: @buttonDisabledEdge;
}
/* The stepper's halves are transparent hit areas; StepperSpinBox draws one
   centred pair of chevrons over them, as the launcher does. */
QDoubleSpinBox { padding: 2px 18px 2px 7px; }
QAbstractSpinBox::up-button,
QAbstractSpinBox::down-button {
    subcontrol-origin: border;
    width: 16px;
    height: 11px;
    border: 0;
    background: transparent;
}
QAbstractSpinBox::up-button { subcontrol-position: top right; }
QAbstractSpinBox::down-button { subcontrol-position: bottom right; }
QAbstractSpinBox::up-arrow,
QAbstractSpinBox::down-arrow { image: none; width: 0; height: 0; border: 0; }
/* A focused field's edge is the accent, inside the ring useEditorStyle
   draws around it. A pop-up button takes focus from Tab alone, and shows
   it by that ring only, as macOS's. */
QListWidget:focus,
QTextEdit:focus,
QPlainTextEdit:focus,
QLineEdit:focus,
QDoubleSpinBox:focus {
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
/* The proposal's script and diff, in fixed-width type so its columns line up. */
QTextEdit#proposalDiff { font-family: Menlo, Monaco, monospace; font-size: 11px; color: @textSecondary; }
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
    padding: 0 22px 0 2px;
    min-height: 22px;
    max-height: 22px;
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
QPushButton#chatSendButton {
    border-radius: 8px;
    padding: 0;
    font-size: 19px;
    min-width: 30px;
    max-width: 30px;
    min-height: 30px;
    max-height: 30px;
}
QToolButton#abortButton { border: 1px solid @assistantBorder; border-radius: 7px; padding: 2px 9px; }

QGroupBox {
    background: @group;
    border: 1px solid @groupBorder;
    border-radius: 8px;
    margin-top: 22px;
    padding: 8px 8px 8px;
}
QGroupBox::title {
    subcontrol-origin: margin;
    subcontrol-position: top left;
    left: 2px;
    top: 2px;
    padding: 0;
    color: @textSecondary;
    font-size: 13px;
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
/* A 6px handle on no track, as macOS's scrollers. */
QScrollBar:vertical, QScrollBar:horizontal { background: transparent; border: 0; margin: 0; }
QScrollBar:vertical { width: 10px; }
QScrollBar:horizontal { height: 10px; }
QScrollBar::handle:vertical, QScrollBar::handle:horizontal {
    background: @scrollHandle;
    border: 0;
    border-radius: 3px;
    margin: 2px;
    min-height: 24px;
    min-width: 24px;
}
QScrollBar::handle:vertical:hover, QScrollBar::handle:horizontal:hover { background: @scrollHandleHover; }
QScrollBar::add-line, QScrollBar::sub-line { width: 0; height: 0; border: 0; background: transparent; }
QScrollBar::add-page, QScrollBar::sub-page { background: transparent; }
/* Where two scroll bars meet: nothing, not a framed square. */
QAbstractScrollArea::corner { background: transparent; border: 0; }
QStatusBar { background: @statusBar; color: @statusText; border-top: 1px solid @divider; font-size: 10px; min-height: 20px; max-height: 20px; }
QStatusBar::item { border: 0; }
QStatusBar QLabel { color: @statusText; font-size: 10px; padding: 0 8px; border-left: 1px solid @statusDivider; }
QStatusBar QLabel#statusConnection[state="ready"] { color: @success; }
QStatusBar QLabel#statusConnection[state="connecting"] { color: @warning; }
QStatusBar QLabel#statusConnection[state="disconnected"] { color: @error; }
/* Banners are bars under the toolbar, edge to edge, with a hairline below. */
QFrame#connectionBanner,
QFrame#errorBanner { background: @errorSurface; border: 0; border-bottom: 1px solid @errorBorder; border-radius: 0; }
QToolBar#toolPalette { background: @chrome; border: 0; border-right: 1px solid @divider; spacing: 2px; padding: 4px 2px; }
QToolBar#toolPalette QToolButton {
    background: transparent;
    border: 0;
    border-radius: 6px;
    min-width: 28px;
    min-height: 24px;
    max-height: 24px;
    padding: 2px;
}
QToolBar#toolPalette QToolButton:hover { background: @hover; }
/* The chosen tool: a quiet rounded chip; its symbol is the accent. */
QToolBar#toolPalette QToolButton:checked { background: @toolSelected; }
QListWidget#projectBin { background: @well; border: 0; }
QListWidget#projectBin::item { color: @text; border-radius: 3px; padding: 4px; }
QListWidget#projectBin::item:selected { background: @selectionInactive; color: @selectionInactiveText; }
QWidget#sourceMonitor { background: @panel; }
QFrame#askPrompt { background: @raised; border: 1px solid @assistant; border-radius: 8px; }
QLabel#askTitle { color: @assistantText; font-size: 12px; font-weight: 600; }
QTextEdit#askInput { background: @field; border: 1px solid @fieldBorder; border-radius: 5px; padding: 4px; color: @text; }
QTextEdit#askInput:focus { border-color: @focusRing; }
/* The agent's bar is the window's chrome; a purple stripe at its start and
   its label say whose it is. Accept is the accent's default action. */
QFrame#aiBar { background: @chrome; border: 0; border-left: 3px solid @assistant; border-bottom: 1px solid @divider; }
QLabel#aiBarText { color: @assistantText; font-size: 11px; font-weight: 600; }
QFrame#aiBar QToolButton { color: @buttonText; padding: 2px 10px; border-radius: 5px; }
QFrame#aiBar QToolButton#aiAccept {
    background: qlineargradient(x1: 0, y1: 0, x2: 0, y2: 1, stop: 0 @accentTop, stop: 1 @accent);
    border-color: @accent;
    border-top-color: @accentTop;
    color: @onAccent;
    font-weight: 600;
}
QFrame#aiBar QToolButton#aiAccept:hover { background: @accentTop; }
QFrame#aiBar QToolButton#aiAccept:pressed { background: @accentPressed; border-color: @accentPressed; }
/* Kerf's suggestions: purple pills sized to their labels, in the agent's
   colours rather than a push button's bezel. */
QPushButton#nextStep {
    color: @assistantText;
    background: @assistantSurface;
    border: 1px solid @assistantBorder;
    border-radius: 11px;
    padding: 2px 10px;
    min-height: 16px;
    max-height: 16px;
    font-size: 11px;
}
QPushButton#nextStep:hover,
QPushButton#nextStep:pressed { background: @assistantHover; border-color: @assistantBorder; }
QFrame#aiBar QToolButton:disabled { color: @buttonDisabledText; background: @buttonDisabled; border-color: @buttonDisabledEdge; }

QFrame#divergedBanner { background: @warningSurface; border: 0; border-bottom: 1px solid @warningBorder; border-radius: 0; }
QLabel#divergedTitle { color: @warningTitle; font-size: 12px; font-weight: 600; }
QLabel#divergedMessage { color: @warningText; font-size: 12px; }
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
QStatusBar QToolButton#syncChip[state="syncing"] { color: @accentText; }
QStatusBar QToolButton#syncChip[state="live"] { color: @success; }
QStatusBar QToolButton#syncChip[state="conflict"],
QStatusBar QToolButton#syncChip[state="failed"] { color: @error; }
QWidget#presetsPanel { background: @panel; }
QLabel#presetsTitle { color: @textSecondary; font-size: 11px; font-weight: 600; }
QListWidget#presetList { background: @well; border: 0; padding: 2px; }
QListWidget#presetList::item { padding: 6px; border-radius: 4px; }
QListWidget#presetList::item:selected { background: @selectionInactive; color: @selectionInactiveText; }
QToolButton#presetsRefresh { background: transparent; border: 0; padding: 0 5px; }

QCheckBox::indicator,
QRadioButton::indicator {
    width: 12px;
    height: 12px;
    background: @checkFill;
    border: 1px solid @checkBorder;
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
    background: @checkFill;
    border: 1px solid @checkBorder;
    border-radius: 3px;
}
QDialog#publishDialog QCheckBox::indicator:checked { background: @accent; border-color: @accent; }
QDialog#publishDialog QCheckBox::indicator:disabled { background: @fieldDisabled; border-color: @borderSubtle; }
)QSS"), theme);
}

// Docked panels, layered over the dock manager's own style sheet: title
// strips with the panel names as tabs. The current tab merges into its
// panel; the focused panel shows only in its tab's label and a hairline
// accent under it, as in Logic and Final Cut, not in a rule across the strip.
//
// Tabs show their close button only under the pointer (Workspace shows and
// hides it). The dock manager's sheet draws a black close button, and a
// white one on the focused tab (which it fills with the highlight); here no
// tab is filled, so every tab takes the editor's symbol in the glyph colour.
inline QString dockStyleSheet(const EditorTheme &theme) {
    return themedStyleSheet(QStringLiteral(R"QSS(
ads--CDockContainerWidget { background: @window; }
ads--CDockContainerWidget > QSplitter { padding: 0; }
/* As specific as the dock manager's Linux sheet, which names the container. */
ads--CDockContainerWidget ads--CDockSplitter::handle { background: @dockSeparator; }
ads--CDockContainerWidget ads--CDockSplitter::handle:hover { background: @dockSeparatorHover; }
ads--CDockAreaWidget { background: @panel; border: 0; }
ads--CDockAreaTitleBar { background: @panelTitle; border-bottom: 1px solid @divider; min-height: 26px; max-height: 26px; }
ads--CDockAreaWidget[focused="true"] ads--CDockAreaTitleBar { background: @panelTitle; border-bottom: 1px solid @divider; }
ads--CDockAreaTabBar { background: @panelTitle; }
ads--CDockWidgetTab { background: @panelTitle; border: 0; border-right: 1px solid @divider; padding: 0 8px; }
ads--CDockWidgetTab QLabel { color: @tabInactiveText; font-size: 11px; font-weight: 600; }
ads--CDockWidgetTab[activeTab="true"] { background: @panelTitleActive; }
ads--CDockWidgetTab[activeTab="true"] QLabel { color: @text; }
ads--CDockWidgetTab[focused="true"] { background: @panelTitleActive; border-right: 1px solid @divider; border-bottom: 1px solid @accent; }
ads--CDockWidgetTab[focused="true"] QLabel { color: @textStrong; }
/* A panel whose body is a white well (the bin, the assistant) has a white
   current tab, so the tab still merges into it. */
ads--CDockWidgetTab[well="true"][activeTab="true"],
ads--CDockWidgetTab[well="true"][focused="true"] { background: @well; }
ads--CDockWidgetTab > #tabCloseButton,
ads--CDockWidgetTab[focused="true"] > #tabCloseButton {
    qproperty-icon: @closeMark, @closeMarkDisabled disabled;
    qproperty-iconSize: 12px 12px;
    background: transparent;
    border: 0;
    border-radius: 3px;
    padding: 1px;
}
ads--CDockWidgetTab > #tabCloseButton:hover,
ads--CDockWidgetTab[focused="true"] > #tabCloseButton:hover { background: @hover; }
ads--CDockWidgetTab > #tabCloseButton:pressed,
ads--CDockWidgetTab[focused="true"] > #tabCloseButton:pressed { background: @pressed; }
#tabsMenuButton { qproperty-icon: @tabsMenuMark; qproperty-iconSize: 12px 12px; }
ads--CDockWidget { background: @panel; border: 0; }
/* A title strip's actions are borderless symbols and plain secondary text,
   20px tall with 3px clear above and below, as Logic and Final Cut draw
   panel headers. The details (frame size, track count) are disabled
   actions, so they read as text, not as dead buttons. */
ads--CTitleBarButton {
    color: @textSecondary;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 5px;
    padding: 0 5px;
    margin: 3px 1px;
    min-height: 18px;
    max-height: 18px;
    font-size: 11px;
}
ads--CTitleBarButton:hover { background: @hover; border-color: transparent; }
ads--CTitleBarButton:pressed,
ads--CTitleBarButton:checked { background: @pressed; border-color: transparent; }
ads--CTitleBarButton:disabled { color: @textSecondary; background: transparent; border-color: transparent; }
ads--CFloatingDockContainer { background: @panel; }
)QSS"), theme);
}

} // namespace kerf
