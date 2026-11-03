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
// names, @checkMark with the check box's tick (in `ink2`), @popUpChevrons (and its
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
        {QStringLiteral("surround"), &EditorTheme::surround},
        {QStringLiteral("surroundDim"), &EditorTheme::surroundDim},
        {QStringLiteral("card"), &EditorTheme::card},
        {QStringLiteral("on"), &EditorTheme::on},
        {QStringLiteral("hover2"), &EditorTheme::hover2},
        {QStringLiteral("ink"), &EditorTheme::ink},
        {QStringLiteral("ink2"), &EditorTheme::ink2},
        {QStringLiteral("ink3"), &EditorTheme::ink3},
        {QStringLiteral("dis"), &EditorTheme::dis},
        {QStringLiteral("track"), &EditorTheme::track},
        {QStringLiteral("track2"), &EditorTheme::track2},
        {QStringLiteral("line"), &EditorTheme::line},
        {QStringLiteral("line2"), &EditorTheme::line2},
        {QStringLiteral("lineh"), &EditorTheme::lineh},
        {QStringLiteral("accentHover"), &EditorTheme::accentHover},
        {QStringLiteral("accentInk"), &EditorTheme::accentInk},
        {QStringLiteral("accentWash"), &EditorTheme::accentWash},
        {QStringLiteral("pos"), &EditorTheme::pos},
        {QStringLiteral("neg"), &EditorTheme::neg},
        {QStringLiteral("warn"), &EditorTheme::warn},
        {QStringLiteral("statusText"), &EditorTheme::statusText},
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
            {QStringLiteral("checkMark"), {checkMarkImagePath, &EditorTheme::ink2}},
            {QStringLiteral("popUpChevrons"), {popUpChevronsImagePath, &EditorTheme::ink2}},
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

// The editor's application style sheet, flat and dark (mockup-spec 0.2-0.5):
// one graphite ramp, hairlines at white 8/14/22%, and the blue accent for
// focus, the default action and Kerf's own work. Controls are 24pt tall
// (28 in the toolbar) with 6pt radii; menus and tooltips are cards with a
// line2 inset; fields sit on `raised` with a line2 inset and the accent in
// focus. Scroll bars are hidden, as the design hides them; the widgets that
// scroll draw their own fades. Nothing is boxed twice, nothing is bevelled,
// and there are no gradients. Type is UI 12/16 for controls, 11/16 for
// meta, 13/20 for body and 14/20 for panel titles; mono is JetBrains Mono
// (else Menlo). Every objectName selector widgets set is kept.
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
/* Tooltip (0.5): 24pt, a card with a line2 inset, 6pt corners. */
QToolTip {
    color: @tooltipText;
    background: @tooltip;
    border: 1px solid @tooltipBorder;
    border-radius: 6px;
    padding: 3px 8px;
    font-size: 12px;
}
/* A pop-up button's menu is a menu (0.5): a card, 8pt corners, rows drawn
   by PopupItemDelegate. Its window is translucent, so the corners clip. */
QComboBox QAbstractItemView {
    background: @menu;
    color: @menuText;
    border: 1px solid @menuBorder;
    border-radius: 8px;
    padding: 4px;
    outline: 0;
}
QComboBox QAbstractItemView::item { min-height: 28px; padding: 0; border: 0; }

QMenuBar {
    background: @chrome;
    border-bottom: 1px solid @divider;
    padding: 2px 6px;
}
QMenuBar::item {
    padding: 3px 8px;
    border-radius: 6px;
}
QMenuBar::item:selected { background: @raised; }
/* Menus (0.5): a card with a line2 inset, 8pt corners, 4pt padding; rows
   28pt with 6pt corners, `raised` under the pointer. They show no symbols
   (see useEditorStyle). */
QMenu {
    background: @menu;
    border: 1px solid @menuBorder;
    border-radius: 8px;
    padding: 4px;
    /* Qt still sets a menu's text past its commands' symbols, by their
       size and 4px more; this leaves the 4px. */
    icon-size: 0px;
}
QMenu::item {
    padding: 6px 18px 6px 22px;
    min-height: 16px;
    border-radius: 6px;
    color: @menuText;
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
QMenu::right-arrow { image: @menuChevron; width: 6px; height: 10px; right: 8px; }
QMenu::right-arrow:selected { image: @menuChevronSelected; }
QMenu::right-arrow:disabled { image: @menuChevronDisabled; }
QMenu::separator { height: 1px; background: @menuSeparator; margin: 4px 0; }
QMenu::item:selected { background: @menuSelection; color: @menuSelectionText; }
QMenu::item:disabled { color: @textDisabled; background: transparent; }

/* The toolbar groups its commands with space, as the design does. The main
   toolbar paints its own plate over this (see FaceplateToolBar). */
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
/* Seams: a 1pt line. */
QSplitter::handle { background: @dockSeparator; }
QSplitter::handle:hover { background: @dockSeparatorHover; }
QSplitter::handle:horizontal { width: 1px; }
QSplitter::handle:vertical { height: 1px; }

/* Buttons (0.5 .btn, secondary): 24pt, 6pt corners, the raised fill with a
   line2 inset, UI 500 12; hover2 and lineh under the pointer, `on` while
   held. They take focus from Tab alone, and show it as the accent ring
   outside the bezel (see useEditorStyle). Icon buttons (.ib) are 24pt
   symbols with no bezel: `raised` under the pointer, `on` when on. */
QPushButton {
    color: @buttonText;
    background: @button;
    border: 1px solid @buttonEdge;
    border-radius: 6px;
    padding: 0px 10px;
    min-height: 22px;
    max-height: 22px;
    font-weight: 500;
    outline: 0;
}
QToolButton {
    color: @text;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 6px;
    padding: 0px 3px;
    min-height: 22px;
    max-height: 22px;
    outline: 0;
}
QPushButton:hover { background: @buttonHover; border-color: @borderStrong; }
QToolButton:hover { background: @raised; border-color: transparent; }
QPushButton:pressed { background: @buttonPressed; border-color: @borderStrong; }
QToolButton:pressed { background: @on; border-color: transparent; }
QToolButton:checked { background: @on; border-color: @line2; }
QPushButton:disabled {
    color: @buttonDisabledText;
    background: @buttonDisabled;
    border-color: @buttonDisabledEdge;
}
QToolButton:disabled { color: @textDisabled; background: transparent; border-color: transparent; }
/* A dialog's buttons are at least 64pt wide. */
QDialogButtonBox QPushButton { min-width: 44px; }
/* The Inspector's mask tools are bare 24pt symbols, as its + and ⋯ are: no
   bezel, `raised` under the pointer. */
QToolButton#maskTool,
QToolButton#maskTool:disabled { background: transparent; border: 1px solid transparent; border-radius: 6px; padding: 0; min-width: 22px; max-width: 22px; min-height: 22px; max-height: 22px; }
QToolButton#maskTool:hover { background: @raised; border-color: transparent; }
QToolButton#maskTool:pressed { background: @on; border-color: transparent; }
/* The chosen mask tool and the selected mask: the on fill, under the
   pointer too, `raised` while held. */
QToolButton#maskTool:checked:enabled,
QToolButton#maskTool:checked:enabled:hover,
QToolButton#maskName:checked:enabled { background: @accentTint; border-color: transparent; }
QToolButton#maskTool:checked:enabled:pressed,
QToolButton#maskName:checked:enabled:pressed { background: @accentTintPressed; }
/* A colour well: its swatch fills a small bezel. */
QPushButton[colorWell="true"] { padding: 0px; font-weight: 400; }
/* Toolbar controls: icon buttons, 24pt in today's toolbar (the 40pt
   toolbar of mockup-spec 1 makes them .t28). */
QToolBar QToolButton {
    background: transparent;
    border-color: transparent;
    min-width: 20px;
    min-height: 22px;
    max-height: 22px;
    padding: 0px 3px;
}
QToolBar QToolButton:hover { background: @raised; border-color: transparent; }
QToolBar QToolButton:pressed { background: @on; border-color: transparent; }
QToolBar QToolButton:disabled { color: @textDisabled; background: transparent; border-color: transparent; }
/* The conversation's switch: while its panel is open it is on. */
QToolBar QToolButton#chatToggle { padding: 0px 6px; min-height: 18px; max-height: 18px; }
QToolBar QToolButton#chatToggle:checked:enabled { background: @accentTint; border-color: transparent; }
QToolBar QToolButton#chatToggle:checked:enabled:pressed { background: @accentTintPressed; }
/* The primary button (.btn.p): the accent, white text, accent-hover under
   the pointer. */
QPushButton#primaryButton,
QDialogButtonBox QPushButton:default,
QPushButton#loadFromDisk {
    background: @accent;
    border-color: @accent;
    color: @onAccent;
    font-weight: 500;
}
QPushButton#primaryButton:hover,
QDialogButtonBox QPushButton:default:hover,
QPushButton#loadFromDisk:hover { background: @accentHover; border-color: @accentHover; }
QPushButton#primaryButton:pressed,
QDialogButtonBox QPushButton:default:pressed,
QPushButton#loadFromDisk:pressed { background: @accentPressed; border-color: @accentPressed; }
QPushButton#primaryButton:disabled,
QDialogButtonBox QPushButton:default:disabled {
    color: @buttonDisabledText;
    background: @buttonDisabled;
    border-color: @buttonDisabledEdge;
}
QPushButton#dangerButton { color: @neg; }
QPushButton#dangerButton:hover { background: @buttonHover; }
/* Transport controls are icon buttons on the panel. */
QToolButton#transportButton,
QPushButton#transportButton {
    background: transparent;
    border: 1px solid transparent;
    border-radius: 6px;
    color: @textSecondary;
    padding: 0px 3px;
    min-height: 22px;
    max-height: 22px;
    min-width: 16px;
    font-size: 11px;
    font-weight: 400;
}
QToolButton#transportButton:disabled { background: transparent; border-color: transparent; color: @textDisabled; }
QToolButton#transportButton:hover,
QPushButton#transportButton:hover {
    background: @raised;
    border-color: transparent;
    color: @textStrong;
}
QToolButton#transportButton:pressed,
QPushButton#transportButton:pressed { background: @on; border-color: transparent; }
QToolButton#abortButton {
    color: @text;
    background: transparent;
    border-color: transparent;
}

QLabel#eyebrow { color: @textSecondary; font-size: 11px; font-weight: 500; letter-spacing: 0px; }
QLabel#muted { color: @textSecondary; font-size: 11px; }
QLabel#chatHint { color: @textSecondary; font-size: 11px; padding: 6px 2px 2px; }
/* The Inspector (5): labels, units, counts and summaries are secondary;
   values the body ink, or the accent ink once moved from their defaults. */
QLabel#inspectorLabel, QLabel#inspectorQuiet, QLabel#inspectorNote, QLabel#sectionCount,
QLabel#sectionSummary, QLabel#effectSummary, QLabel#effectGroup, QLabel#effectMissing,
QLabel#captionHint, QLabel#masksHint { color: @textSecondary; }
QLabel#sectionTitle, QLabel#effectName { color: @text; }
/* Before the switched-off rule, which wins over it. */
QLineEdit[inspector="true"][changed="true"] { color: @accentText; }
QLabel#effectName[off="true"],
QLabel#colorHex[off="true"],
QLineEdit[inspector="true"][off="true"] { color: @textSecondary; }
QFrame#inspectorRule, QFrame#inspectorScrollEdge { background: @divider; border: 0; }
QFrame#inspectorDropLine { background: @textStrong; border: 0; }
QLineEdit#inspectorNameEditor {
    background: @inspectorField;
    border: 1px solid @inspectorField;
    border-radius: 6px;
}
QLineEdit[inspector="true"], QPlainTextEdit[inspector="true"], QLineEdit#scrubEditor {
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
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
/* A link in the Inspector (the caption's Ask Kerf). */
QPushButton[inspectorLink="true"][quiet="true"] { color: @textSecondary; font-weight: 400; }
QPushButton[underlineOnHover="true"] { font-size: 12px; font-weight: 500; }
QPushButton[underlineOnHover="true"]:hover { text-decoration: underline; background: transparent; border: 0; }
QPushButton[inspectorLink="true"]:hover { color: @textStrong; background: transparent; }
/* A mask's name: words, on the on fill while selected. */
QToolButton#maskName { background: transparent; border-color: transparent; padding: 2px 6px; font-weight: 500; }
/* The window's title (1): UI 500 13, centred in the toolbar. */
QLabel#projectTitle { color: @text; font-size: 13px; font-weight: 500; padding: 0; }
QLabel#projectSubtitle { color: @textSecondary; font-size: 11px; padding: 0; }
QToolButton#eyebrow { text-align: left; background: transparent; border: 0; color: @textSecondary; padding: 2px 0; }
QToolButton#eyebrow:hover { color: @text; background: transparent; }
/* A disclosure: a chevron and a label, no bezel. */
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

/* Fields (0.5): `raised` with a line2 inset, 6pt corners; the accent in
   focus. Multi-line fields keep a 3pt bezel. */
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
/* Single-line fields and pop-ups are 24pt. */
QLineEdit,
QDoubleSpinBox,
QComboBox {
    background: @field;
    border: 1px solid @fieldBorder;
    border-radius: 6px;
    padding: 0px 8px;
    min-height: 22px;
    max-height: 22px;
    selection-background-color: @textSelection;
    selection-color: @text;
}
QListWidget:hover,
QTextEdit:hover,
QPlainTextEdit:hover,
QLineEdit:hover,
QDoubleSpinBox:hover,
QComboBox:hover { border-color: @borderStrong; }
/* Pop-up buttons and steppers: chevrons on the control, with no bevelled
   arrow cells. A pop-up button's height is its content's, with no padding
   above or below: Qt reads that padding as its menu's margin. */
QComboBox { padding: 0 24px 0 8px; combobox-popup: 1; }
QComboBox::drop-down {
    subcontrol-origin: padding;
    subcontrol-position: center right;
    width: 22px;
    border: 0;
    background: transparent;
}
QComboBox::down-arrow { image: @popUpChevrons; width: 10px; height: 14px; border: 0; }
QComboBox::down-arrow:disabled { image: @popUpChevronsDisabled; }
/* A pop-up button is a secondary button's bezel with chevrons; a combo box
   that takes typing (the Assistant's Model) stays a field. */
QComboBox:!editable {
    color: @buttonText;
    background: @button;
    border-color: @buttonEdge;
}
QComboBox:!editable:hover {
    background: @buttonHover;
    border-color: @borderStrong;
}
QComboBox:!editable:disabled {
    color: @buttonDisabledText;
    background: @buttonDisabled;
    border-color: @buttonDisabledEdge;
}
/* The Inspector's pop-ups: clear, no bezel, 24pt as its fields; a choice
   moved from its default in the accent ink. */
QComboBox[inspector="true"],
QComboBox[inspector="true"]:!editable,
QComboBox[inspector="true"]:!editable:hover {
    color: @text;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
    min-height: 22px;
    max-height: 22px;
}
QComboBox[inspector="true"][changed="true"],
QComboBox[inspector="true"][changed="true"]:!editable { color: @accentText; }
QComboBox[inspector="true"]:focus,
QComboBox[inspector="true"]:!editable:focus { border-color: @focusRing; }
QComboBox[inspector="true"]:!editable:disabled { color: @textDisabled; background: transparent; border-color: transparent; }
/* The stepper's halves are transparent hit areas; StepperSpinBox draws one
   centred pair of chevrons over them. */
QDoubleSpinBox { padding: 0px 18px 0px 8px; }
QAbstractSpinBox::up-button,
QAbstractSpinBox::down-button {
    subcontrol-origin: border;
    width: 16px;
    height: 12px;
    border: 0;
    background: transparent;
}
QAbstractSpinBox::up-button { subcontrol-position: top right; }
QAbstractSpinBox::down-button { subcontrol-position: bottom right; }
QAbstractSpinBox::up-arrow,
QAbstractSpinBox::down-arrow { image: none; width: 0; height: 0; border: 0; }
/* A focused field's edge is the accent, inside the ring useEditorStyle
   draws around it. A pop-up button shows focus by that ring only. */
QListWidget:focus,
QTextEdit:focus,
QPlainTextEdit:focus,
QLineEdit:focus,
QDoubleSpinBox:focus {
    border-color: @focusRing;
    background: @field;
}
/* No dotted focus frame on the current item: the selection shows it. */
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
/* A group is a raised card with no frame. */
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
    font-weight: 500;
}
/* Progress: the track, the track2 fill. */
QProgressBar { background: @track; border: 0; border-radius: 1px; height: 2px; max-height: 2px; }
QProgressBar::chunk { background: @track2; border-radius: 1px; }

/* Tabs (Settings) as the segmented control (0.5): a raised track, the
   chosen one on the on fill. */
QTabWidget::pane { border: 0; background: transparent; }
QTabBar { background: transparent; }
QTabBar::tab {
    color: @tabText;
    background: transparent;
    padding: 2px 8px;
    border: 1px solid transparent;
    border-radius: 4px;
}
QTabBar::tab:hover { color: @text; }
QTabBar::tab:selected {
    color: @tabSelectedText;
    background: @tabSelected;
    border-color: @tabSelectedBorder;
    font-weight: 500;
}

/* Slider (0.5 .sl): a 2pt track, a 10pt ink knob. */
QSlider::groove:horizontal { background: @sliderGroove; border: 0; height: 2px; border-radius: 1px; }
QSlider::sub-page:horizontal { background: @track2; border-radius: 1px; }
QSlider::handle:horizontal { background: @sliderHandle; border: 0; width: 10px; height: 10px; margin: -4px 0; border-radius: 5px; }
/* Scroll bars are hidden (the design hides every one); the widgets that
   scroll draw their own fades. The handle rules stay for a widget that
   shows a bar of its own. */
QScrollBar:vertical, QScrollBar:horizontal { background: transparent; border: 0; margin: 0; }
QScrollBar:vertical { width: 0px; }
QScrollBar:horizontal { height: 0px; }
QScrollBar::handle:vertical, QScrollBar::handle:horizontal {
    background: @scrollHandle;
    border: 0;
    border-radius: 3px;
    margin: 0;
    min-height: 24px;
    min-width: 24px;
}
QScrollBar::handle:vertical:hover, QScrollBar::handle:horizontal:hover,
QScrollBar::handle:vertical:pressed, QScrollBar::handle:horizontal:pressed { background: @scrollHandleHover; }
QScrollBar::add-line, QScrollBar::sub-line { width: 0; height: 0; border: 0; background: transparent; }
QScrollBar::add-page, QScrollBar::sub-page { background: transparent; }
QAbstractScrollArea::corner { background: transparent; border: 0; }
/* Status bar (9): 24pt on chrome, a top hairline, UI 11 meta. */
QStatusBar { background: @statusBar; color: @statusText; border-top: 1px solid @divider; font-size: 11px; min-height: 23px; max-height: 23px; }
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
QToolBar#toolPalette QToolButton:hover { background: @raised; }
/* The chosen tool: the on fill. */
QToolBar#toolPalette QToolButton:checked { background: @toolSelected; }
/* The bin's tiles draw themselves. */
QListWidget#projectBin { background: transparent; border: 0; padding: 0; }
QListWidget#projectBin::item,
QListWidget#projectBin::item:hover,
QListWidget#projectBin::item:selected,
QListWidget#projectBin::item:selected:focus { background: transparent; border: 0; padding: 0; }
QLineEdit#binSearch { background: @inspectorField; border: 1px solid @line2; border-radius: 6px; min-height: 22px; max-height: 22px; padding: 0 6px; color: @text; }
QLineEdit#binSearch:focus { background: @inspectorField; border-color: @focusRing; }
QWidget#sourceMonitor { background: @panel; }
/* Ask Kerf is a popover (0.5 menu): a card with a line2 inset. */
QFrame#askPrompt { background: @card; border: 1px solid @menuBorder; border-radius: 8px; }
QLabel#askTitle { color: @text; font-size: 13px; font-weight: 500; }
QTextEdit#askInput { background: @field; border: 1px solid @fieldBorder; border-radius: 6px; padding: 4px; color: @text; }
QTextEdit#askInput:focus { border-color: @focusRing; }
/* The agent's bar is the window's chrome: a quiet label, chips for Details,
   Stop and Reject, and Accept, the accent's default action. */
QFrame#aiBar { background: @chrome; border: 0; border-bottom: 1px solid @divider; }
QLabel#aiBarText { color: @textSecondary; font-size: 12px; font-weight: 500; }
QFrame#aiBar QToolButton {
    color: @chatChipText;
    background: @chatChip;
    border: 0;
    border-radius: 6px;
    padding: 0 10px;
    min-height: 22px;
    max-height: 22px;
    font-size: 12px;
    font-weight: 500;
}
QFrame#aiBar QToolButton:hover,
QFrame#aiBar QToolButton:pressed { background: @chatChipHover; }
QFrame#aiBar QToolButton#aiAccept { background: @accent; color: @onAccent; font-weight: 500; }
QFrame#aiBar QToolButton#aiAccept:hover { background: @accentHover; }
QFrame#aiBar QToolButton#aiAccept:pressed { background: @accentPressed; border-color: @accentPressed; }
QFrame#aiBar QToolButton:disabled { color: @textDisabled; background: @chatChip; }

/* Kerf's conversation (4): ChatPanel paints its rounded card, and on it
   nothing is boxed. Names UI 500 13, body 13/20, meta 11 ink3. */
QLabel#chatTitle { color: @chatText; font-size: 14px; font-weight: 500; }
QLabel#chatCount { color: @chatSecondary; font-size: 11px; font-family: "JetBrains Mono", Menlo, monospace; }
QLabel#chatName { color: @chatText; font-size: 13px; font-weight: 500; }
QLabel#chatTime,
QLabel#chatPinned,
QLabel#chatStatus,
QLabel#chatActivity,
QLabel#chatChangeMore { color: @chatSecondary; font-size: 11px; }
QLabel#chatSection { color: @chatSecondary; font-size: 11px; font-weight: 500; }
QLabel#chatErrorTag { color: @errorText; font-size: 11px; font-weight: 500; }
QLabel#chatBody,
QLabel#proposalSummary { color: @chatText; font-size: 13px; }
QLabel#proposalBarSummary { color: @chatText; font-size: 12px; font-weight: 500; }
QLabel#chatReplyText { color: @chatText; font-size: 12px; }
QScrollArea#chatScroll, QWidget#chatContent { background: transparent; border: 0; }
/* The Inspector's and the conversation's lists scroll with no bar. */
QScrollArea#inspectorScroll QScrollBar:vertical { width: 0px; background: transparent; margin: 0; }
QScrollArea#inspectorScroll QScrollBar::handle:vertical { background: transparent; border-radius: 3px; margin: 0; min-height: 24px; }
QScrollArea#inspectorScroll[active="true"] QScrollBar::handle:vertical { background: transparent; }
QScrollArea#inspectorScroll[active="true"] QScrollBar::handle:vertical:hover,
QScrollArea#inspectorScroll[active="true"] QScrollBar::handle:vertical:pressed { background: transparent; }
QScrollArea#chatScroll QScrollBar:vertical { width: 0px; background: transparent; margin: 0; }
QScrollArea#chatScroll QScrollBar::handle:vertical { background: transparent; border-radius: 2px; margin: 0; min-height: 24px; }
QScrollArea#chatScroll[hovered="true"] QScrollBar::handle:vertical { background: @chatScrollHandle; }
QScrollArea#chatScroll[hovered="true"] QScrollBar::handle:vertical:hover,
QScrollArea#chatScroll[hovered="true"] QScrollBar::handle:vertical:pressed { background: @chatScrollHandleHover; }
QFrame#scrollEdge { background: transparent; border: 0; }
QFrame#scrollEdge[scrolled="true"] { background: @divider; }
QFrame#proposalBox,
QFrame#proposalBar { background: transparent; border: 0; }
QFrame#proposalRule { background: @divider; border: 0; }
/* The proposal's script and diff, in mono so its columns line up. */
QTextEdit#proposalDiff { background: transparent; border: 0; font-family: "JetBrains Mono", Menlo, monospace; font-size: 11px; color: @chatSecondary; }
/* The footer: a hairline over it, then the composer. */
QFrame#chatComposer { background: transparent; border: 0; border-top: 1px solid @divider; border-radius: 0; }
QTextEdit#chatInput { background: transparent; border: 0; padding: 0; color: @chatText; font-size: 13px; }
QTextEdit#chatInput:focus { background: transparent; border: 0; }
QTextEdit#chatInput:disabled { background: transparent; color: @textDisabled; border: 0; }
/* The model: a ghost button with a chevron (0.5 .ghost-b). Its field draws
   in one colour whether or not the pointer is over it. */
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
/* Pills: Send and Accept filled with the accent, Reject and Stop on
   `raised`, Cancel bare text; none has an outline. */
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
    font-weight: 500;
}
QPushButton#chatPillFilled,
QPushButton#chatSendButton { color: @onAccent; background: @accent; }
QPushButton#chatPillFilled:hover,
QPushButton#chatSendButton:hover { background: @accentHover; }
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
/* The pinned proposal's Accept and Reject, a size up. */
QFrame#proposalBox QPushButton#chatPillFilled,
QFrame#proposalBox QPushButton#chatPillTonal { border-radius: 18px; min-height: 36px; max-height: 36px; padding: 0 18px; }
QFrame#proposalBox[narrow="true"] QPushButton#chatPillFilled,
QFrame#proposalBox[narrow="true"] QPushButton#chatPillTonal { padding: 0 12px; }
QFrame#proposalBar QPushButton#chatPillFilled,
QFrame#proposalBar QPushButton#chatPillTonal { padding: 0 12px; }
QPushButton#chatPillFilled:disabled,
QPushButton#chatSendButton:disabled,
QPushButton#chatPillTonal:disabled,
QPushButton#abortButton:disabled { color: @textDisabled; background: @pillTonal; }
/* A comment's quiet actions: words with `raised` under the pointer. */
QPushButton#chatAction {
    color: @chatSecondary;
    background: transparent;
    border: 0;
    border-radius: 6px;
    padding: 0 8px;
    min-height: 24px;
    max-height: 24px;
    font-size: 12px;
    font-weight: 400;
}
QPushButton#chatAction:hover,
QPushButton#chatAction:pressed { color: @chatText; background: @chatChip; }
/* Kerf's next steps and the welcome's examples: chips on `raised`. */
QPushButton#nextStep {
    color: @chatChipText;
    background: @chatChip;
    border: 0;
    border-radius: 6px;
    padding: 0 12px;
    min-height: 28px;
    max-height: 28px;
    font-size: 12px;
    font-weight: 500;
}
QPushButton#nextStep:hover,
QPushButton#nextStep:pressed { background: @chatChipHover; }
/* On the timeline's bar, the same chips at its controls' height. */
QFrame#aiBar QPushButton#nextStep { min-height: 22px; max-height: 22px; border-radius: 6px; padding: 0 10px; font-size: 12px; }
QFrame#activityPopup { background: @menu; border: 1px solid @menuBorder; border-radius: 8px; }
QListWidget#activityList { background: transparent; border: 0; padding: 0; }

QFrame#divergedBanner { background: @warningSurface; border: 0; border-bottom: 1px solid @warningBorder; border-radius: 0; }
QLabel#divergedTitle { color: @warningTitle; font-size: 12px; font-weight: 500; }
QLabel#divergedMessage { color: @warningText; font-size: 12px; }
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
QStatusBar QToolButton#syncChip:hover { color: @textStrong; background: @raised; }
QStatusBar QToolButton#syncChip[state="syncing"] { color: @accentText; }
QStatusBar QToolButton#syncChip[state="live"] { color: @success; }
QStatusBar QToolButton#syncChip[state="conflict"],
QStatusBar QToolButton#syncChip[state="failed"] { color: @error; }
QWidget#presetsPanel,
QWidget#effectsPanel { background: @panel; }
QLabel#presetsTitle { color: @textSecondary; font-size: 11px; font-weight: 500; }
QListWidget#presetList { background: @well; border: 0; padding: 2px; }
QListWidget#presetList::item { padding: 6px; border-radius: 4px; }
QListWidget#presetList::item:selected { background: @selectionInactive; color: @selectionInactiveText; }
QToolButton#presetsRefresh { background: transparent; border: 0; padding: 0 5px; }

/* Check box (0.5 .cbx): 14pt, 4pt corners, a 16% white outline; on, the on
   fill and an ink2 check. */
QCheckBox::indicator,
QRadioButton::indicator {
    width: 12px;
    height: 12px;
    background: transparent;
    border: 1px solid @checkBorder;
}
QCheckBox::indicator { border-radius: 4px; }
QRadioButton::indicator { border-radius: 7px; }
QCheckBox::indicator:checked { background: @on; border-color: @checkBorder; image: @checkMark; }
QRadioButton::indicator:checked {
    background: qradialgradient(cx: 0.5, cy: 0.5, radius: 0.5, fx: 0.5, fy: 0.5,
                                stop: 0 @ink2, stop: 0.3 @ink2, stop: 0.42 @on, stop: 1 @on);
    border-color: @checkBorder;
}
QCheckBox::indicator:disabled,
QRadioButton::indicator:disabled { background: transparent; border-color: @borderSubtle; }
QCheckBox::indicator:checked:disabled { background: @raised; border-color: @borderSubtle; }
QRadioButton::indicator:checked:disabled {
    background: qradialgradient(cx: 0.5, cy: 0.5, radius: 0.5, fx: 0.5, fy: 0.5,
                                stop: 0 @textDisabled, stop: 0.3 @textDisabled, stop: 0.42 @raised, stop: 1 @raised);
    border-color: @borderSubtle;
}

QDialog#publishDialog QCheckBox { spacing: 8px; }
QDialog#publishDialog QCheckBox::indicator {
    width: 12px;
    height: 12px;
    background: transparent;
    border: 1px solid @checkBorder;
    border-radius: 4px;
}
QDialog#publishDialog QCheckBox::indicator:checked { background: @on; border-color: @checkBorder; image: @checkMark; }
QDialog#publishDialog QCheckBox::indicator:disabled { background: transparent; border-color: @borderSubtle; }
)QSS"), theme);
}

// Docked panels, layered over the dock manager's own style sheet. Each
// area's title bar is a 28pt strip on chrome with a bottom hairline and the
// panel names in UI 500 12 (mockup 2, 4.2, 7.1): no tab look, no boxes and
// no underline, so an area with one panel shows its strip and title only;
// in an area with several, the current name is ink and the others ink3.
// Tabs never show a close button (Workspace hides it); here every tab takes
// the editor's symbol in the glyph colour.
inline QString dockStyleSheet(const EditorTheme &theme) {
    return themedStyleSheet(QStringLiteral(R"QSS(
ads--CDockContainerWidget { background: @window; }
ads--CDockContainerWidget > QSplitter { padding: 0; }
/* As specific as the dock manager's Linux sheet, which names the container. */
ads--CDockContainerWidget ads--CDockSplitter::handle { background: @dockSeparator; }
ads--CDockContainerWidget ads--CDockSplitter::handle:hover { background: @dockSeparatorHover; }
ads--CDockAreaWidget { background: @panel; border: 0; }
ads--CDockAreaTitleBar { background: @chrome; border: 0; border-bottom: 1px solid @divider; min-height: 27px; max-height: 27px; padding-left: 8px; }
ads--CDockAreaWidget[focused="true"] ads--CDockAreaTitleBar { background: @chrome; border-bottom: 1px solid @divider; }
ads--CDockAreaTabBar { background: transparent; }
ads--CDockWidgetTab { background: transparent; border: 0; margin: 0 4px; padding: 0 4px; }
ads--CDockWidgetTab QLabel { color: @ink3; font-size: 12px; font-weight: 500; }
ads--CDockWidgetTab[focused="true"] { background: transparent; border: 0; }
ads--CDockWidgetTab[focused="true"] QLabel { color: @ink3; }
ads--CDockWidgetTab[activeTab="true"] { background: transparent; border: 0; }
ads--CDockWidgetTab[activeTab="true"] QLabel { color: @text; }
ads--CDockWidgetTab > #tabCloseButton,
ads--CDockWidgetTab[focused="true"] > #tabCloseButton {
    qproperty-icon: @closeMark, @closeMarkDisabled disabled;
    qproperty-iconSize: 12px 12px;
    background: transparent;
    border: 0;
    border-radius: 4px;
    padding: 1px;
}
ads--CDockWidgetTab > #tabCloseButton:hover,
ads--CDockWidgetTab[focused="true"] > #tabCloseButton:hover { background: @raised; }
ads--CDockWidgetTab > #tabCloseButton:pressed,
ads--CDockWidgetTab[focused="true"] > #tabCloseButton:pressed { background: @on; }
#tabsMenuButton { qproperty-icon: @tabsMenuMark; qproperty-iconSize: 12px 12px; }
ads--CDockWidget { background: @panel; border: 0; }
/* A strip's actions are icon buttons (.ib, 20-24pt) and meta text; the
   details (frame size, track count) are disabled actions, so they read as
   text, not as dead buttons. */
ads--CTitleBarButton {
    color: @textSecondary;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 6px;
    padding: 0 4px;
    margin: 2px 1px;
    min-height: 20px;
    max-height: 20px;
    font-size: 11px;
}
ads--CTitleBarButton:hover { background: @raised; border-color: transparent; }
ads--CTitleBarButton:pressed,
ads--CTitleBarButton:checked { background: @on; border-color: transparent; }
ads--CTitleBarButton:disabled { color: @textSecondary; background: transparent; border-color: transparent; }
/* A chosen mask tool in the Program strip is on, as in the Inspector. */
ads--CTitleBarButton#maskRectangle:checked:enabled,
ads--CTitleBarButton#maskEllipse:checked:enabled,
ads--CTitleBarButton#maskPen:checked:enabled { background: @accentTint; border-color: transparent; }
ads--CTitleBarButton#maskRectangle:checked:enabled:pressed,
ads--CTitleBarButton#maskEllipse:checked:enabled:pressed,
ads--CTitleBarButton#maskPen:checked:enabled:pressed { background: @accentTintPressed; }
/* The Timeline's track heights are one segmented control the timeline
   paints: 20x24 each, edge to edge. */
ads--CTitleBarButton#trackHeightMini,
ads--CTitleBarButton#trackHeightMedium,
ads--CTitleBarButton#trackHeightLarge,
ads--CTitleBarButton#trackHeightFit {
    background: transparent;
    border: 0;
    padding: 0;
    margin: 0;
    min-width: 20px;
    max-width: 20px;
    min-height: 24px;
    max-height: 24px;
}
ads--CFloatingDockContainer { background: @panel; }
/* The timeline's strip: its details after its name. */
QLabel#timelineDetail { color: @textSecondary; font-size: 11px; padding: 0 0 0 8px; }
QWidget#timelineZoom QToolButton { background: transparent; border: 0; padding: 0; min-height: 20px; max-height: 20px; }
)QSS"), theme);
}

} // namespace kerf
