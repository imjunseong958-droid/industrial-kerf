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
// names, @checkMark with the check box's tick (in `onAccent`), @popUpChevrons (and its
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
        {QStringLiteral("accentTintPressed"), &EditorTheme::accentTintPressed},
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
        {QStringLiteral("inspectorField"), &EditorTheme::inspectorField},
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
        {QStringLiteral("chatSurface"), &EditorTheme::chatSurface},
        {QStringLiteral("chatText"), &EditorTheme::chatText},
        {QStringLiteral("chatSecondary"), &EditorTheme::chatSecondary},
        {QStringLiteral("chatChip"), &EditorTheme::chatChip},
        {QStringLiteral("chatChipHover"), &EditorTheme::chatChipHover},
        {QStringLiteral("chatChipText"), &EditorTheme::chatChipText},
        {QStringLiteral("chatScrollHandle"), &EditorTheme::chatScrollHandle},
        {QStringLiteral("chatScrollHandleHover"), &EditorTheme::chatScrollHandleHover},
        {QStringLiteral("pillTonal"), &EditorTheme::pillTonal},
        {QStringLiteral("pillTonalText"), &EditorTheme::pillTonalText},
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
            {QStringLiteral("checkMark"), {checkMarkImagePath, &EditorTheme::onAccent}},
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
        if (const auto drawn = images.constFind(name); drawn != images.cend()) {
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

// The editor's application style sheet: the project launcher's cream and
// espresso, with Kerf's purple for focus, the default action and Kerf's own
// work. Density follows desktop edit suites: docked panels with title
// strips and a status bar, on one type scale (11, 12, 13, 14, 16 and 20px)
// and a 4px grid. Nothing is boxed twice: a chosen tool or mode is a tonal
// wash with no outline, and cards sit on their panel without a frame.
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
/* The Inspector's mask tools are bare 24px symbols, as its + and ⋯ are: no
   bezel, a wash under the pointer. */
QToolButton#maskTool,
QToolButton#maskTool:disabled { background: transparent; border: 1px solid transparent; border-radius: 6px; padding: 0; min-width: 22px; max-width: 22px; min-height: 22px; max-height: 22px; }
QToolButton#maskTool:hover { background: @hover; border-color: transparent; }
QToolButton#maskTool:pressed { background: @pressed; border-color: transparent; }
QToolButton#maskTool:checked:enabled:hover { background: @accentTint; border-color: transparent; }
/* The chosen mask tool and the selected mask: a quiet accent wash with no
   outline, under the pointer too, darkening while held as every bezel does.
   The tool's symbol is the accent, which reads on either wash at 3:1. */
QToolButton#maskTool:checked:enabled,
QToolButton#maskName:checked:enabled { background: @accentTint; border-color: transparent; }
QToolButton#maskTool:checked:enabled:pressed,
QToolButton#maskName:checked:enabled:pressed { background: @accentTintPressed; }
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
/* The conversation's switch: as tall as the pop-ups and buttons before it,
   and while its column is open it looks as a chosen mode in a title strip
   does: a quiet accent wash with no outline, its symbol the accent, under
   the pointer too, darkening while held. */
QToolBar QToolButton#chatToggle { padding: 0px 6px; min-height: 18px; max-height: 18px; }
QToolBar QToolButton#chatToggle:checked:enabled { background: @accentTint; border-color: transparent; }
QToolBar QToolButton#chatToggle:checked:enabled:pressed { background: @accentTintPressed; }
QPushButton#primaryButton {
    background: qlineargradient(x1: 0, y1: 0, x2: 0, y2: 1, stop: 0 @accentTop, stop: 1 @accent);
    border-color: @accent;
    border-top-color: @accentTop;
    color: @onAccent;
    font-weight: 600;
}
QPushButton#primaryButton:hover { background: @accentTop; }
QPushButton#primaryButton:pressed { background: @accentPressed; border-color: @accentPressed; }
QPushButton#primaryButton:disabled {
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
/* Transport controls are symbols on the panel: a round wash under the
   pointer, no outline. */
QToolButton#transportButton,
QPushButton#transportButton {
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
    color: @text;
    padding: 2px 4px;
    min-height: 16px;
    max-height: 16px;
    min-width: 16px;
    font-size: 11px;
}
QToolButton#transportButton:disabled { background: transparent; border-color: transparent; }
QToolButton#transportButton:hover,
QPushButton#transportButton:hover {
    background: @hover;
    border-color: transparent;
    color: @textStrong;
}
QToolButton#transportButton:pressed,
QPushButton#transportButton:pressed { background: @pressed; border-color: transparent; }
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
QToolButton#abortButton {
    color: @text;
    background: transparent;
    border-color: transparent;
}

QLabel#eyebrow { color: @textSecondary; font-size: 12px; font-weight: 500; letter-spacing: 0px; }
QLabel#muted { color: @textSecondary; font-size: 11px; }
QLabel#chatHint { color: @textSecondary; font-size: 11px; padding: 6px 2px 2px; }
/* The Inspector. A fill means a value can be changed; plain text cannot.
   Labels, units, counts and summaries are secondary; values and titles are
   the body ink; the only purple is the focus ring. */
QLabel#inspectorLabel, QLabel#inspectorQuiet, QLabel#inspectorNote, QLabel#sectionCount,
QLabel#sectionSummary, QLabel#effectSummary, QLabel#effectGroup, QLabel#effectMissing,
QLabel#captionHint, QLabel#masksHint { color: @textSecondary; }
QLabel#sectionTitle, QLabel#effectName { color: @text; }
QLabel#effectName[off="true"] { color: @textSecondary; }
QFrame#inspectorRule, QFrame#inspectorScrollEdge { background: @divider; border: 0; }
QFrame#inspectorDropLine { background: @textStrong; border: 0; }
QLineEdit[inspector="true"], QPlainTextEdit[inspector="true"],
QLineEdit#scrubEditor, QLineEdit#inspectorNameEditor {
    background: @inspectorField;
    border: 1px solid @inspectorField;
    border-radius: 5px;
}
QLineEdit[inspector="true"]:focus, QPlainTextEdit[inspector="true"]:focus,
QLineEdit#scrubEditor, QLineEdit#inspectorNameEditor { border-color: @focusRing; }
QLineEdit#scrubEditor { padding: 0px 7px; min-height: 22px; max-height: 22px; }
/* Add effect, Reveal in Project, "6 more": words, no bezel. */
QPushButton[inspectorLink="true"] {
    background: transparent;
    border: 0;
    color: @text;
    font-size: 12px;
    font-weight: 500;
    padding: 0px;
    min-height: 24px;
    max-height: 24px;
    text-align: left;
}
/* A link in the Inspector (the caption's Ask Kerf): underlined under the pointer. */
QPushButton[underlineOnHover="true"] { font-size: 13px; font-weight: 400; }
QPushButton[underlineOnHover="true"]:hover { text-decoration: underline; background: transparent; border: 0; }
QPushButton[inspectorLink="true"][quiet="true"] { color: @textSecondary; font-weight: 400; }
QPushButton[inspectorLink="true"]:hover { color: @textStrong; background: transparent; }
/* A mask's name: words, washed in the accent tint while selected. */
QToolButton#maskName { background: transparent; border-color: transparent; padding: 2px 6px; font-weight: 600; }
/* The window's title, 13px semibold as macOS's and the launcher's. */
QLabel#projectTitle { color: @text; font-size: 13px; font-weight: 600; padding: 0; }
QLabel#projectSubtitle { color: @textSecondary; font-size: 11px; padding: 0; }
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
/* The Inspector's pop-ups: its field fill, no bezel, 24px as its fields. */
QComboBox[inspector="true"],
QComboBox[inspector="true"]:!editable,
QComboBox[inspector="true"]:!editable:hover {
    color: @text;
    background: @inspectorField;
    border: 1px solid @inspectorField;
    border-radius: 5px;
    min-height: 22px;
    max-height: 22px;
}
QComboBox[inspector="true"]:focus,
QComboBox[inspector="true"]:!editable:focus { border-color: @focusRing; }
QComboBox[inspector="true"]:!editable:disabled { color: @textDisabled; background: @inspectorField; border-color: @inspectorField; }
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
QListWidget#presetList::item:selected:focus { background: @selection; color: @selectionText; }
QListWidget#mediaList,
QListWidget#effectsCards,
QScrollArea#mediaList { background: transparent; border: 0; padding: 0; }
QWidget#mediaGrid { background: transparent; }
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
/* A group is a paper card a step lighter than the dialog, with no frame. */
QGroupBox {
    background: @raised;
    border: 0;
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
    font-size: 12px;
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
    border-color: transparent;
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
QScrollBar::handle:vertical:hover, QScrollBar::handle:horizontal:hover,
QScrollBar::handle:vertical:pressed, QScrollBar::handle:horizontal:pressed { background: @scrollHandleHover; }
QScrollBar::add-line, QScrollBar::sub-line { width: 0; height: 0; border: 0; background: transparent; }
QScrollBar::add-page, QScrollBar::sub-page { background: transparent; }
/* Where two scroll bars meet: nothing, not a framed square. */
QAbstractScrollArea::corner { background: transparent; border: 0; }
QStatusBar { background: @statusBar; color: @statusText; border-top: 1px solid @divider; font-size: 11px; min-height: 20px; max-height: 20px; }
QStatusBar::item { border: 0; }
QStatusBar QLabel { color: @statusText; font-size: 11px; padding: 0 8px; border-left: 1px solid @statusDivider; }
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
/* The bin's tiles draw themselves (a ring round the chosen thumbnail). */
QListWidget#projectBin { background: transparent; border: 0; padding: 0; }
QListWidget#projectBin::item,
QListWidget#projectBin::item:hover,
QListWidget#projectBin::item:selected,
QListWidget#projectBin::item:selected:focus { background: transparent; border: 0; padding: 0; }
QLineEdit#binSearch { background: @inspectorField; border: 0; border-radius: 6px; min-height: 28px; max-height: 28px; padding: 0 6px; color: @text; }
QLineEdit#binSearch:focus { background: @inspectorField; border: 0; }
QWidget#sourceMonitor { background: @panel; }
/* Ask Kerf is a popover: a paper card with a quiet edge, not a purple callout. */
QFrame#askPrompt { background: @raised; border: 1px solid @menuBorder; border-radius: 8px; }
QLabel#askTitle { color: @assistantText; font-size: 12px; font-weight: 600; }
QTextEdit#askInput { background: @field; border: 1px solid @fieldBorder; border-radius: 5px; padding: 4px; color: @text; }
QTextEdit#askInput:focus { border-color: @focusRing; }
/* The agent's bar is the window's chrome, no stripe and no callout: a quiet
   label, tonal chips for Details, Stop and Reject, and Accept, the
   accent's default action. */
QFrame#aiBar { background: @chrome; border: 0; border-bottom: 1px solid @divider; }
QLabel#aiBarText { color: @textSecondary; font-size: 12px; font-weight: 600; }
QFrame#aiBar QToolButton {
    color: @chatChipText;
    background: @chatChip;
    border: 0;
    border-radius: 8px;
    padding: 0 10px;
    min-height: 22px;
    max-height: 22px;
    font-size: 12px;
    font-weight: 500;
}
QFrame#aiBar QToolButton:hover,
QFrame#aiBar QToolButton:pressed { background: @chatChipHover; }
QFrame#aiBar QToolButton#aiAccept { background: @accent; color: @onAccent; font-weight: 600; }
QFrame#aiBar QToolButton#aiAccept:hover { background: @accentTop; }
QFrame#aiBar QToolButton#aiAccept:pressed { background: @accentPressed; border-color: @accentPressed; }
QFrame#aiBar QToolButton:disabled { color: @textDisabled; background: @chatChip; }

/* Kerf's conversation, as a video's comments panel: ChatPanel paints its
   rounded surface, and on it nothing is boxed. Every turn is a comment
   row, Kerf's purple marks only its avatar, its expanders and Send. */
QLabel#chatTitle { color: @chatText; font-size: 16px; font-weight: 700; }
QLabel#chatCount { color: @chatSecondary; font-size: 14px; }
QLabel#chatName { color: @chatText; font-size: 12px; font-weight: 600; }
QLabel#chatTime,
QLabel#chatPinned,
QLabel#chatStatus,
QLabel#chatActivity,
QLabel#chatChangeMore { color: @chatSecondary; font-size: 12px; }
QLabel#chatSection { color: @chatSecondary; font-size: 12px; font-weight: 600; }
QLabel#chatErrorTag { color: @errorText; font-size: 12px; font-weight: 600; }
QLabel#chatBody,
QLabel#proposalSummary { color: @chatText; font-size: 14px; }
QLabel#proposalBarSummary { color: @chatText; font-size: 13px; font-weight: 600; }
QLabel#chatReplyText { color: @chatText; font-size: 13px; }
QScrollArea#chatScroll, QWidget#chatContent { background: transparent; border: 0; }
/* The list's scroll bar shows only while the pointer is over the panel
   (ChatPanel sets "hovered"), as a comments panel's: a 4px pill. */
QScrollArea#chatScroll QScrollBar:vertical { width: 8px; background: transparent; margin: 4px 2px; }
QScrollArea#chatScroll QScrollBar::handle:vertical { background: transparent; border-radius: 2px; margin: 0; min-height: 24px; }
QScrollArea#chatScroll[hovered="true"] QScrollBar::handle:vertical { background: @chatScrollHandle; }
QScrollArea#chatScroll[hovered="true"] QScrollBar::handle:vertical:hover,
QScrollArea#chatScroll[hovered="true"] QScrollBar::handle:vertical:pressed { background: @chatScrollHandleHover; }
QFrame#scrollEdge { background: transparent; border: 0; }
QFrame#scrollEdge[scrolled="true"] { background: @divider; }
QFrame#proposalBox,
QFrame#proposalBar { background: transparent; border: 0; }
QFrame#proposalRule { background: @divider; border: 0; }
/* The proposal's script and diff, in fixed-width type so its columns line up. */
QTextEdit#proposalDiff { background: transparent; border: 0; font-family: Menlo, Monaco, monospace; font-size: 11px; color: @chatSecondary; }
/* The footer: a hairline over it, then the reader's avatar and a one-line
   field over its underline (ChatPanel draws that, in the accent in focus). */
QFrame#chatComposer { background: transparent; border: 0; border-top: 1px solid @divider; border-radius: 0; }
QTextEdit#chatInput { background: transparent; border: 0; padding: 0; color: @chatText; font-size: 14px; }
QTextEdit#chatInput:focus { background: transparent; border: 0; }
QTextEdit#chatInput:disabled { background: transparent; color: @textDisabled; border: 0; }
/* The model: a small text pop-up. Its field draws in one colour whether or
   not the pointer is over it, so that colour reads on the hover wash too. */
QComboBox#agentModel {
    background: transparent;
    border: 0;
    border-radius: 6px;
    padding: 0 18px 0 4px;
    min-height: 24px;
    max-height: 24px;
    font-size: 12px;
    color: @chatText;
}
QComboBox#agentModel:hover { background: @chatChipHover; }
QComboBox#agentModel:disabled { color: @chatSecondary; background: transparent; }
QComboBox#agentModel QLineEdit { background: transparent; border: 0; padding: 0; color: @chatText; }
/* Pills, 32px with round ends: Send and Accept filled with the accent,
   Reject and Stop tonal, Cancel bare text; none has an outline. */
QPushButton#chatPillFilled,
QPushButton#chatSendButton,
QPushButton#chatPillTonal,
QPushButton#abortButton,
QPushButton#chatPillText {
    border: 0;
    border-radius: 16px;
    padding: 0 16px;
    min-height: 32px;
    max-height: 32px;
    font-size: 13px;
    font-weight: 600;
}
QPushButton#chatPillFilled,
QPushButton#chatSendButton { color: @onAccent; background: @accent; }
QPushButton#chatPillFilled:hover,
QPushButton#chatSendButton:hover { background: @accentTop; }
QPushButton#chatPillFilled:pressed,
QPushButton#chatSendButton:pressed { background: @accentPressed; }
QPushButton#chatPillTonal,
QPushButton#abortButton { color: @pillTonalText; background: @pillTonal; }
QPushButton#chatPillTonal:hover,
QPushButton#abortButton:hover,
QPushButton#chatPillTonal:pressed,
QPushButton#abortButton:pressed { background: @chatChipHover; }
QPushButton#chatPillText { color: @chatText; background: transparent; padding: 0 12px; }
QPushButton#chatPillText:hover,
QPushButton#chatPillText:pressed { background: @chatChip; }
/* The pinned proposal's Accept and Reject, a size up: 36px, round ends. */
QFrame#proposalBox QPushButton#chatPillFilled,
QFrame#proposalBox QPushButton#chatPillTonal { border-radius: 18px; min-height: 36px; max-height: 36px; padding: 0 18px; }
/* The compact bar's, a little tighter, so its summary keeps room beside them. */
QFrame#proposalBar QPushButton#chatPillFilled,
QFrame#proposalBar QPushButton#chatPillTonal { padding: 0 12px; }
QPushButton#chatPillFilled:disabled,
QPushButton#chatSendButton:disabled,
QPushButton#chatPillTonal:disabled,
QPushButton#abortButton:disabled { color: @textDisabled; background: @pillTonal; }
/* A comment's quiet actions: text that takes a round wash under the pointer. */
QPushButton#chatAction {
    color: @chatSecondary;
    background: transparent;
    border: 0;
    border-radius: 12px;
    padding: 0 10px;
    min-height: 24px;
    max-height: 24px;
    font-size: 12px;
    font-weight: 500;
}
QPushButton#chatAction:hover,
QPushButton#chatAction:pressed { color: @chatText; background: @chatChip; }
/* Kerf's next steps and the welcome's examples: tonal chips, no outline. */
QPushButton#nextStep {
    color: @chatChipText;
    background: @chatChip;
    border: 0;
    border-radius: 8px;
    padding: 0 12px;
    min-height: 28px;
    max-height: 28px;
    font-size: 13px;
    font-weight: 500;
}
QPushButton#nextStep:hover,
QPushButton#nextStep:pressed { background: @chatChipHover; }
/* On the timeline's bar, the same chips at its controls' height. */
QFrame#aiBar QPushButton#nextStep { min-height: 22px; max-height: 22px; border-radius: 6px; padding: 0 10px; font-size: 12px; }
QFrame#activityPopup { background: @menu; border: 1px solid @menuBorder; border-radius: 6px; }
QListWidget#activityList { background: transparent; border: 0; padding: 0; }

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
    font-size: 11px;
    padding: 0 8px;
    min-height: 0;
}
QStatusBar QToolButton#syncChip:hover { color: @textStrong; background: @hover; }
QStatusBar QToolButton#syncChip[state="syncing"] { color: @accentText; }
QStatusBar QToolButton#syncChip[state="live"] { color: @success; }
QStatusBar QToolButton#syncChip[state="conflict"],
QStatusBar QToolButton#syncChip[state="failed"] { color: @error; }
QWidget#presetsPanel,
QWidget#effectsPanel { background: @panel; }
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
/* Text tabs, as the structure the owner chose: no strip and no boxes, the
   panel's own surface under a row of 13px names, the chosen one in the
   strongest ink over a 2px underline of it, a hairline under the row. */
ads--CDockAreaTitleBar { background: @panel; border: 0; border-bottom: 1px solid @divider; min-height: 31px; max-height: 31px; padding-left: 8px; }
ads--CDockAreaWidget[focused="true"] ads--CDockAreaTitleBar { background: @panel; border-bottom: 1px solid @divider; }
ads--CDockAreaTabBar { background: transparent; }
ads--CDockWidgetTab { background: transparent; border: 0; border-bottom: 2px solid transparent; margin: 0 6px; padding: 2px 2px 0 2px; }
ads--CDockWidgetTab QLabel { color: @tabInactiveText; font-size: 13px; font-weight: 500; }
/* The dock manager's focus mark is not drawn (a stale one would read as a
   second chosen tab): only the current tab is underlined. */
ads--CDockWidgetTab[focused="true"] { background: transparent; border: 0; border-bottom: 2px solid transparent; }
ads--CDockWidgetTab[focused="true"] QLabel { color: @tabInactiveText; }
ads--CDockWidgetTab[activeTab="true"] { background: transparent; border: 0; border-bottom: 2px solid @textStrong; }
ads--CDockWidgetTab[activeTab="true"] QLabel { color: @textStrong; }
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
/* A chosen mode in a title strip (the Program panel's mask tools, the
   Timeline's track height) looks as the chosen mask tool does in Effect
   Controls: a quiet accent wash with no outline, its symbol the accent,
   darkening while held. */
ads--CTitleBarButton#maskRectangle:checked:enabled,
ads--CTitleBarButton#maskEllipse:checked:enabled,
ads--CTitleBarButton#maskPen:checked:enabled,
ads--CTitleBarButton#trackHeightMini:checked:enabled,
ads--CTitleBarButton#trackHeightMedium:checked:enabled,
ads--CTitleBarButton#trackHeightLarge:checked:enabled { background: @accentTint; border-color: transparent; }
ads--CTitleBarButton#maskRectangle:checked:enabled:pressed,
ads--CTitleBarButton#maskEllipse:checked:enabled:pressed,
ads--CTitleBarButton#maskPen:checked:enabled:pressed,
ads--CTitleBarButton#trackHeightMini:checked:enabled:pressed,
ads--CTitleBarButton#trackHeightMedium:checked:enabled:pressed,
ads--CTitleBarButton#trackHeightLarge:checked:enabled:pressed { background: @accentTintPressed; }
ads--CFloatingDockContainer { background: @panel; }
/* The timeline's tab row: its details after its name, then (on the right)
   its zoom's − slider +. */
QLabel#timelineDetail { color: @textSecondary; font-size: 12px; padding: 2px 0 0 10px; }
QWidget#timelineZoom QToolButton { background: transparent; border: 0; border-radius: 4px; padding: 0; min-height: 20px; max-height: 20px; }
QWidget#timelineZoom QToolButton:hover { background: @hover; }
QWidget#timelineZoom QToolButton:pressed { background: @pressed; }
)QSS"), theme);
}

} // namespace kerf
