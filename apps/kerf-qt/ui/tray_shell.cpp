#include "tray_shell.h"

#include "components.h"

#include <QAbstractButton>
#include <QAction>
#include <QHBoxLayout>
#include <QLabel>
#include <QPainter>
#include <QPainterPath>
#include <QVBoxLayout>
#include <QVariantAnimation>

#include <algorithm>

namespace kerf::shell {

QLabel *text(const QString &content, const tokens::TypeStyle &style, QRgb ink) {
    auto *label = new QLabel(content);
    label->setFont(tokens::font(style));
    QPalette palette = label->palette();
    palette.setColor(QPalette::WindowText, tokens::color(ink));
    label->setPalette(palette);
    setTextStyle(label, style, ink);
    return label;
}

void setTextStyle(QLabel *label, const tokens::TypeStyle &style, QRgb ink) {
    QPalette palette = label->palette();
    palette.setColor(QPalette::WindowText, tokens::color(ink));
    label->setPalette(palette);
    label->setStyleSheet(QStringLiteral("QLabel { color: %1; font-size: %2px; font-weight: %3; background: transparent; }")
                             .arg(tokens::color(ink).name(QColor::HexRgb))
                             .arg(style.px)
                             .arg(style.weight));
}

void followAction(QAbstractButton *button, const QAction *action, const QString &blocked) {
    if (!button) return;
    const bool available = action && action->isEnabled();
    button->setEnabled(available && blocked.isEmpty());
    // The registry's reason first (offline, say), then what holds it back here.
    button->setToolTip(!action ? QString() : available && !blocked.isEmpty() ? blocked : action->toolTip());
}

} // namespace kerf::shell
