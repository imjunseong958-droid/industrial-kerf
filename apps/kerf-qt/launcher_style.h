#pragma once

#include "editor_theme.h"

#include <QColor>
#include <QString>

#include <utility>

namespace kerf {

// The project launcher, the first window Kerf shows, shares the editor's
// palette: antique cream paper in light, warm espresso in dark, and Kerf's
// own purple (not the system accent) for selection, switches, and the
// default button. Painted parts (tiles, sidebar symbols, switches) read the
// same tokens.
struct LauncherTheme {
    bool dark = false;
    QColor accent;
    QColor accentTop;
    QColor accentPressed;
    QColor onAccent; // Text on the filled accent (the default button), 4.5:1.
    // Secondary text on `selection`: `selectionText` held back only as far
    // as it still reads at 4.5:1.
    QColor onAccentSecondary;
    QColor content;
    QColor titleText;
    QColor titleRule;
    QColor sidebar;
    QColor sidebarDivider;
    QColor text;
    // Secondary text, and the Details disclosure's chevron.
    QColor secondary;
    QColor disabledText;
    QColor separator;
    QColor field;
    QColor fieldBorder;
    QColor fieldDisabled;
    QColor focusRing;
    QColor textSelection;
    // A chosen row in a list that has focus, and in one that has not (the
    // sidebar's rows among them), with the detail text on the latter.
    QColor selection;
    QColor selectionText; // Text and symbols on `selection` (in dark, not `onAccent`).
    QColor selectionInactive;
    QColor secondaryOnSelection;
    // Push buttons and pop-ups: a fill with a lighter top edge and a darker
    // bottom one, as macOS draws its bezels.
    QColor button;
    QColor buttonEdge;
    QColor buttonTop;
    QColor buttonBottom;
    QColor buttonPressed;
    // A disabled button is the same bezel at half strength.
    QColor buttonDisabled;
    QColor buttonDisabledEdge;
    QColor buttonDisabledText;
    QColor group;
    QColor groupBorder;
    // A format tile: a paper card lighter than the page, lifted by its
    // shadow, with the canvas frame in ink and a + on a disc.
    QColor tile;
    QColor tileShadow;
    QColor tileGlyph;
    QColor badge;
    QColor switchOff;
    QColor switchOffStroke;
    QColor knob;
    QColor knobOn; // The knob on the accent track (dark ink on the dark appearance's mauve).
    QColor knobEdge;
    QColor error;
    QColor success;
    // A scroll bar's handle reads at 3:1 on the content, groups and sidebar
    // it sits on; under the pointer or held, more.
    QColor scrollHandle;
    QColor scrollHandleHover;
    QColor tooltip;
    QColor tooltipText;
};

// `over` laid on `under` at `amount` (0 to 1).
inline QColor launcherMix(const QColor &under, const QColor &over, qreal amount) {
    return QColor::fromRgbF(under.redF() + (over.redF() - under.redF()) * amount,
                            under.greenF() + (over.greenF() - under.greenF()) * amount,
                            under.blueF() + (over.blueF() - under.blueF()) * amount);
}

inline LauncherTheme launcherTheme(bool dark) {
    const auto tone = [dark](const char *light, const char *darkValue) { return cssColor(dark ? darkValue : light); };
    LauncherTheme t;
    t.dark = dark;
    t.accent = tone("#6a3471", "#c39acb");
    t.accentTop = tone("#7a4381", "#cfa9d6");
    t.accentPressed = tone("#562a5c", "#a882b1");
    t.onAccent = tone("#fbf6ec", "#21161f");
    t.onAccentSecondary = tone("#fbf6eca5", "#f4ecdea6");
    t.content = tone("#f4eee2", "#1c1713");
    t.titleText = tone("#2a231d", "#e6dccb");
    t.titleRule = tone("#dcd2bf", "#342b24");
    t.sidebar = tone("#ebe4d5", "#241e19");
    t.sidebarDivider = tone("#dcd2bf", "#342b24");
    t.text = tone("#2a231d", "#f0e7d8");
    t.secondary = tone("#675b4f", "#a89a8a");
    t.disabledText = tone("#a89c8c", "#6d6156");
    t.separator = tone("#e0d7c5", "#342b24");
    t.field = tone("#fbf8f1", "#2a231d");
    t.fieldBorder = tone("#8b7e6d", "#7a6c5e");
    t.fieldDisabled = tone("#f1ebdf", "#221c17");
    t.focusRing = tone("#6a347159", "#c39acb59");
    t.textSelection = tone("#d8c9d2", "#675363");
    t.selection = tone("#6a3471", "#583863");
    t.selectionInactive = tone("#ddd3c0", "#3a3029");
    t.secondaryOnSelection = tone("#5a4e43", "#b8aa99");
    t.button = tone("#fbf8f1", "#3a3029");
    t.buttonEdge = tone("#3c28142e", "#3a3029");
    t.buttonTop = tone("#3c28141a", "#4a3f36");
    t.buttonBottom = tone("#3c28144c", "#2e2620");
    t.buttonPressed = tone("#ebe3d4", "#4a3f36");
    t.buttonDisabled = tone("#f5f0e6", "#2a231d");
    t.buttonDisabledEdge = tone("#3c28141e", "#322a23");
    t.buttonDisabledText = tone("#a89c8c", "#7d7064");
    t.group = tone("#efe8db", "#231d18");
    t.groupBorder = tone("#e0d7c5", "#342b24");
    t.tile = tone("#faf6ee", "#262019");
    t.tileShadow = tone("#3c281428", "#00000073");
    t.tileGlyph = tone("#3d342c", "#e6dccb");
    t.badge = tone("#e9e1d1", "#3a3029");
    t.switchOff = tone("#e2d9c8", "#3a3029");
    t.switchOffStroke = tone("#8b7e6d", "#85776a");
    t.knob = tone("#fffcf6", "#efe6d6");
    t.knobEdge = tone("#281a0c2e", "#0000003c");
    t.error = tone("#a3302b", "#e8877b");
    t.success = tone("#3d6a2e", "#93bf78");
    t.scrollHandle = tone("#3c2a1a8e", "#ffecd26e");
    t.scrollHandleHover = tone("#3c2a1abe", "#ffecd2a0");
    t.tooltip = cssColor("#2f2721");
    t.selectionText = tone("#fbf6ec", "#f4ecde");
    t.knobOn = tone("#fffcf6", "#21161f");
    t.tooltipText = tone("#f3ebdd", "#f4ecde");
    // Guards: each holds for the values above, and keeps holding if one is
    // retuned.
    t.onAccent = readableOn(t.onAccent, t.accent, 4.5);
    t.selectionText = readableOn(t.selectionText, t.selection, 4.5);
    t.onAccentSecondary = readableAlphaOn(t.onAccentSecondary, t.selection, 4.5);
    return t;
}

// A complete style sheet for the launcher and everything in it. The editor's
// application style sheet stays in force for every property this one leaves
// out, so each widget type here sets all it relies on.
inline QString launcherStyleSheet(const LauncherTheme &t) {
    const auto c = [](const QColor &color) {
        return color.alpha() == 255 ? color.name() : color.name(QColor::HexArgb);
    };
    QString sheet = QStringLiteral(R"QSS(
* {
    color: @text;
    font-size: 13px;
}
QDialog#projectLauncher,
QWidget#launcherContent,
QMessageBox { background: @content; }
QWidget#launcherPagesArea,
QWidget#launcherPages,
QWidget#launcherHeadingBox,
QWidget#launcherDetails,
QWidget#launcherDetailsBody,
QWidget#presetSlots { background: transparent; border: 0; }
QToolTip {
    color: @tooltipText;
    background: @tooltip;
    border: 1px solid @tooltip;
    border-radius: 4px;
    padding: 4px 7px;
    font-size: 12px;
}

QLabel { background: transparent; border: 0; padding: 0; }
QLabel#launcherTitle {
    color: @titleText;
    background: @content;
    border: 0;
    border-bottom: 1px solid @titleRule;
    font-size: 13px;
    font-weight: 600;
}
QLabel#launcherHeading { font-size: 13px; font-weight: 600; }
QLabel#launcherSubheading { color: @secondary; font-size: 11px; }
QLabel#launcherHint { color: @secondary; font-size: 12px; }
QLabel#launcherEyebrow { color: @secondary; font-size: 11px; font-weight: 600; }
QLabel#launcherProblem { color: @secondary; font-size: 12px; }
QLabel#launcherProblem[state="error"] { color: @error; }
QLabel#launcherDetailsSummary { color: @secondary; font-size: 12px; }
QLabel#launcherFormLabel { padding-top: 3px; }
QLabel#presetSlotName { font-weight: 600; }
QLabel#presetStartLabel { color: @secondary; font-size: 12px; }
QLabel[fit="none"] { color: @secondary; font-size: 12px; }
QLabel[fit="yes"] { color: @success; font-size: 12px; }
QLabel[fit="no"] { color: @error; font-size: 12px; }

QListWidget,
QListView {
    background: transparent;
    border: 0;
    border-radius: 0;
    padding: 0;
    outline: 0;
    selection-background-color: transparent;
    selection-color: @text;
}
QFrame#launcherSidebarPane {
    background: @sidebar;
    border: 0;
    border-right: 1px solid @sidebarDivider;
}
QListWidget#launcherSidebar {
    background: transparent;
    border: 0;
    padding: 2px 8px 8px 8px;
}
QFrame#launcherSidebarFoot {
    background: transparent;
    border: 0;
    border-top: 1px solid @sidebarDivider;
}
/* Groups are a tone off the page with no frame, so a chosen row inside
   one is the only box. */
QListWidget#presetChoices {
    background: @group;
    border: 0;
    border-radius: 8px;
    padding: 0;
}
QFrame#presetSlotGroup {
    background: @group;
    border: 0;
    border-radius: 8px;
}
QFrame#presetSlot { background: transparent; border: 0; }
QFrame#launcherGroupSeparator {
    background: @separator;
    border: 0;
    margin-left: 14px;
    min-height: 1px;
    max-height: 1px;
}
QWidget#launcherDetailsRow {
    background: transparent;
    border: 0;
    border-top: 1px solid @separator;
}
QFrame#launcherFooter {
    background: @content;
    border: 0;
    border-top: 1px solid @separator;
}

QScrollArea { background: transparent; border: 0; }
QScrollArea > QWidget > QWidget { background: transparent; }
QScrollBar:vertical,
QScrollBar:horizontal { background: transparent; border: 0; margin: 2px; }
QScrollBar:vertical { width: 10px; }
QScrollBar:horizontal { height: 10px; }
QScrollBar::handle:vertical,
QScrollBar::handle:horizontal { background: @scrollHandle; border: 0; border-radius: 3px; min-height: 28px; min-width: 28px; }
QScrollBar::handle:vertical:hover,
QScrollBar::handle:horizontal:hover,
QScrollBar::handle:vertical:pressed,
QScrollBar::handle:horizontal:pressed { background: @scrollHandleHover; }
QScrollBar::add-line,
QScrollBar::sub-line { width: 0; height: 0; border: 0; background: transparent; }
QScrollBar::add-page,
QScrollBar::sub-page { background: transparent; }

/* Its own height, not the editor's 22px button metrics. */
QToolButton#launcherDetailsToggle {
    background: transparent;
    border: 0;
    border-radius: 0;
    padding: 0;
    min-height: 19px;
    max-height: 19px;
    color: @text;
    font-size: 13px;
}

QPushButton,
QComboBox {
    background: @button;
    color: @text;
    border: 1px solid @buttonEdge;
    border-top-color: @buttonTop;
    border-bottom-color: @buttonBottom;
    border-radius: 5px;
    padding: 2px 12px;
    min-height: 16px;
    max-height: 16px;
    font-size: 13px;
    font-weight: 400;
    outline: 0;
}
QPushButton:pressed { background: @buttonPressed; }
QPushButton:disabled,
QComboBox:disabled {
    color: @buttonDisabledText;
    background: @buttonDisabled;
    border-color: @buttonDisabledEdge;
}
QPushButton#launcherChoose:enabled {
    background: qlineargradient(x1: 0, y1: 0, x2: 0, y2: 1, stop: 0 @accentTop, stop: 1 @accent);
    border-color: @accent;
    border-top-color: @accentTop;
    color: @onAccent;
}
QPushButton#launcherChoose { min-width: 58px; }
QPushButton#launcherChoose:pressed { background: @accentPressed; border-color: @accentPressed; }

QLineEdit,
QDoubleSpinBox {
    background: @field;
    color: @text;
    border: 1px solid @fieldBorder;
    border-radius: 5px;
    padding: 2px 7px;
    min-height: 16px;
    max-height: 16px;
    selection-background-color: @textSelection;
    selection-color: @text;
    /* Placeholders carry guidance, so they read at 4.5:1 as the editor's
       do; a disabled field's stay dimmed with the field. */
    placeholder-text-color: @secondary;
}
QLineEdit:focus,
QDoubleSpinBox:focus { border-color: @accent; }
QLineEdit:disabled,
QDoubleSpinBox:disabled { color: @disabledText; background: @fieldDisabled; border-color: @separator; placeholder-text-color: @disabledText; }
QDoubleSpinBox { padding-right: 2px; }
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
QAbstractSpinBox::down-arrow { image: none; border: 0; width: 0; height: 0; }

QComboBox { padding-left: 9px; padding-right: 24px; combobox-popup: 1; }
QComboBox::drop-down {
    subcontrol-origin: padding;
    subcontrol-position: center right;
    width: 22px;
    border: 0;
    background: transparent;
}
QComboBox::down-arrow { image: none; border: 0; width: 0; height: 0; }
QComboBox QAbstractItemView {
    background: @field;
    color: @text;
    border: 1px solid @fieldBorder;
    border-radius: 6px;
    padding: 4px;
    outline: 0;
    selection-background-color: @selection;
    selection-color: @selectionText;
}
QComboBox QAbstractItemView::item { min-height: 22px; padding: 0; border: 0; }
QCheckBox { background: transparent; spacing: 8px; }

QProgressBar {
    background: @separator;
    border: 0;
    border-radius: 2px;
    min-height: 4px;
    max-height: 4px;
}
QProgressBar::chunk { background: @accent; border-radius: 2px; }

QMenu {
    background: @field;
    border: 1px solid @fieldBorder;
    border-radius: 6px;
    padding: 4px;
}
QMenu::item { color: @text; background: transparent; padding: 3px 18px 3px 10px; border-radius: 4px; }
QMenu::item:selected { color: @selectionText; background: @selection; }
QMenu::item:disabled { color: @disabledText; background: transparent; }
QMenu::separator { height: 1px; background: @separator; margin: 4px 6px; }
)QSS");
    // Longer names first, so @accent does not eat @accentTop.
    const std::pair<const char *, QColor> tokens[] = {
        {"@accentPressed", t.accentPressed}, {"@accentTop", t.accentTop}, {"@onAccent", t.onAccent},
        {"@accent", t.accent}, {"@content", t.content}, {"@titleText", t.titleText}, {"@titleRule", t.titleRule},
        {"@sidebarDivider", t.sidebarDivider}, {"@sidebar", t.sidebar}, {"@textSelection", t.textSelection},
        {"@text", t.text}, {"@secondary", t.secondary}, {"@disabledText", t.disabledText},
        {"@separator", t.separator}, {"@fieldBorder", t.fieldBorder}, {"@fieldDisabled", t.fieldDisabled},
        {"@field", t.field}, {"@buttonEdge", t.buttonEdge}, {"@buttonTop", t.buttonTop},
        {"@buttonBottom", t.buttonBottom}, {"@buttonPressed", t.buttonPressed},
        {"@buttonDisabledEdge", t.buttonDisabledEdge}, {"@buttonDisabledText", t.buttonDisabledText},
        {"@buttonDisabled", t.buttonDisabled}, {"@button", t.button},
        {"@groupBorder", t.groupBorder}, {"@group", t.group}, {"@error", t.error}, {"@success", t.success},
        {"@scrollHandleHover", t.scrollHandleHover}, {"@scrollHandle", t.scrollHandle},
        {"@tooltipText", t.tooltipText}, {"@tooltip", t.tooltip}, {"@selectionText", t.selectionText},
        {"@selection", t.selection},
    };
    for (const auto &[token, color] : tokens) sheet.replace(QLatin1String(token), c(color));
    return sheet;
}

} // namespace kerf
