#pragma once

#include "design_tokens.h"
#include "editor_theme.h"

#include <QColor>
#include <QString>

#include <utility>

namespace kerf {

// The project launcher, the first window Kerf shows, draws from the editor's
// graphite ramp and blue accent (design_tokens.h), dark only: the sidebar on
// chrome, the pages on card, groups and tiles on raised, and the accent (not
// the system's) for the default button, focus and a focused list's chosen
// row. Painted parts (tiles, sidebar symbols, switches) read the same tokens.
struct LauncherTheme {
    QColor accent;
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
    // Push buttons and pop-ups: the raised fill with a line2 edge, flat (the
    // top and bottom edges are the same).
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
    // A format tile: a raised card on the page, with the canvas frame in ink2
    // and a + on an `on` disc.
    QColor tile;
    QColor tileShadow;
    QColor tileGlyph;
    QColor badge;
    QColor switchOff;
    QColor switchOffStroke;
    QColor knob;
    QColor knobOn; // The knob on the accent track (white).
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

inline LauncherTheme launcherTheme() {
    namespace tk = tokens;
    const auto c = [](QRgb rgba) { return tk::color(rgba); };
    const auto alpha = [](QRgb rgb, double fraction) {
        QColor color = tk::color(rgb);
        color.setAlpha(tk::alpha(fraction));
        return color;
    };
    // A line as it shows laid on a surface, for the seams the launcher fills.
    const auto laid = [](QRgb under, QRgb line) {
        QColor top = tk::color(line);
        const qreal amount = top.alphaF();
        top.setAlpha(255);
        return QColor::fromRgba(mixColor(tk::color(under), top, amount).rgba());
    };
    LauncherTheme t;
    t.accent = c(tk::accent);
    t.accentPressed = c(tk::accentHover);
    t.onAccent = c(tk::accentText);
    t.onAccentSecondary = alpha(tk::accentText, 0.78);
    t.content = c(tk::card);
    t.titleText = c(tk::ink);
    t.titleRule = laid(tk::card, tk::line);
    t.sidebar = c(tk::chrome);
    t.sidebarDivider = laid(tk::chrome, tk::line);
    t.text = c(tk::ink);
    t.secondary = c(tk::ink2);
    t.disabledText = c(tk::dis);
    t.separator = laid(tk::card, tk::line);
    t.field = c(tk::raised);
    t.fieldBorder = c(tk::line2);
    t.fieldDisabled = c(tk::chrome);
    t.focusRing = c(tk::accent);
    t.textSelection = alpha(tk::accent, 0.45);
    t.selection = c(tk::accent);
    t.selectionInactive = c(tk::on);
    t.secondaryOnSelection = c(tk::ink2);
    t.button = c(tk::raised);
    t.buttonEdge = c(tk::line2);
    t.buttonTop = c(tk::line2);
    t.buttonBottom = c(tk::line2);
    t.buttonPressed = c(tk::on);
    t.buttonDisabled = c(tk::chrome);
    t.buttonDisabledEdge = c(tk::line);
    t.buttonDisabledText = c(tk::dis);
    t.group = c(tk::raised);
    t.groupBorder = c(tk::line);
    t.tile = c(tk::raised);
    t.tileShadow = alpha(0xff000000, 0.45);
    t.tileGlyph = c(tk::ink2);
    t.badge = c(tk::on);
    t.switchOff = c(tk::track);
    t.switchOffStroke = c(tk::line2);
    t.knob = c(tk::ink);
    t.knobOn = c(tk::accentText);
    t.knobEdge = alpha(0xff000000, 0.24);
    t.error = c(tk::neg);
    t.success = c(tk::pos);
    t.scrollHandle = alpha(0xffffffff, 0.40);
    t.scrollHandleHover = alpha(0xffffffff, 0.70);
    t.tooltip = c(tk::card);
    t.selectionText = c(tk::accentText);
    t.tooltipText = c(tk::ink);
    // Guards: each holds for the values above, and keeps holding if one is
    // retuned.
    t.onAccent = readableOn(t.onAccent, t.accent, 4.5);
    t.selectionText = readableOn(t.selectionText, t.selection, 4.5);
    t.onAccentSecondary = readableAlphaOn(t.onAccentSecondary, t.selection, 4.5);
    t.secondaryOnSelection = readableOn(t.secondaryOnSelection, t.selectionInactive, 4.5);
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
    border: 1px solid @fieldBorder;
    border-radius: 6px;
    padding: 3px 8px;
    font-size: 12px;
}

QLabel { background: transparent; border: 0; padding: 0; }
QLabel#launcherTitle {
    color: @titleText;
    background: @content;
    border: 0;
    border-bottom: 1px solid @titleRule;
    font-size: 13px;
    font-weight: 500;
}
QLabel#launcherHeading { font-size: 14px; font-weight: 500; }
QLabel#launcherSubheading { color: @secondary; font-size: 11px; }
QLabel#launcherHint { color: @secondary; font-size: 12px; }
QLabel#launcherEyebrow { color: @secondary; font-size: 11px; font-weight: 500; }
QLabel#launcherProblem { color: @secondary; font-size: 12px; }
QLabel#launcherProblem[state="error"] { color: @error; }
QLabel#launcherDetailsSummary { color: @secondary; font-size: 12px; }
QLabel#launcherFormLabel { padding-top: 3px; }
QLabel#presetSlotName { font-weight: 500; }
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

/* Its own height, not the editor's 24pt button metrics. */
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

/* Buttons and pop-ups (mockup-spec 0.5 .btn): 24pt, 6pt corners, the
   raised fill with a line2 edge, UI 500 12. */
QPushButton,
QComboBox {
    background: @button;
    color: @text;
    border: 1px solid @buttonEdge;
    border-radius: 6px;
    padding: 0px 10px;
    min-height: 22px;
    max-height: 22px;
    font-size: 12px;
    font-weight: 500;
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
    background: @accent;
    border-color: @accent;
    color: @onAccent;
}
QPushButton#launcherChoose { min-width: 42px; }
QPushButton#launcherChoose:pressed { background: @accentPressed; border-color: @accentPressed; }

QLineEdit,
QDoubleSpinBox {
    background: @field;
    color: @text;
    border: 1px solid @fieldBorder;
    border-radius: 6px;
    padding: 0px 8px;
    min-height: 22px;
    max-height: 22px;
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
    height: 12px;
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
    background: @content;
    color: @text;
    border: 1px solid @fieldBorder;
    border-radius: 8px;
    padding: 4px;
    outline: 0;
    selection-background-color: @field;
    selection-color: @text;
}
QComboBox QAbstractItemView::item { min-height: 28px; padding: 0; border: 0; }
QCheckBox { background: transparent; spacing: 8px; }

QProgressBar {
    background: @separator;
    border: 0;
    border-radius: 1px;
    min-height: 2px;
    max-height: 2px;
}
QProgressBar::chunk { background: @accent; border-radius: 1px; }

QMenu {
    background: @content;
    border: 1px solid @fieldBorder;
    border-radius: 8px;
    padding: 4px;
}
QMenu::item { color: @text; background: transparent; padding: 6px 18px 6px 8px; border-radius: 6px; }
QMenu::item:selected { color: @text; background: @field; }
QMenu::item:disabled { color: @disabledText; background: transparent; }
QMenu::separator { height: 1px; background: @separator; margin: 4px 0; }
)QSS");
    // Longer names first, so @accent does not eat @accentPressed.
    const std::pair<const char *, QColor> tokens[] = {
        {"@accentPressed", t.accentPressed}, {"@onAccent", t.onAccent},
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
