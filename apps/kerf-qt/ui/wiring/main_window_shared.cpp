#include "main_window_shared.h"
#include "editor_common.h"
#include "editor_theme.h"

#include <QPainter>
#include <QPainterPath>
#include <QPixmap>
#include <QPolygonF>

namespace kerf::window_parts {

// SF Symbols' scissors on a 20-point grid, in the painter's pen: the
// toolbar's Split and the tool palette's Razor.
static void drawScissors(QPainter &painter) {
    painter.drawEllipse(QPointF(5, 5), 2.5, 2.5);
    painter.drawEllipse(QPointF(5, 15), 2.5, 2.5);
    QPainterPath blades;
    blades.moveTo(7, 6.5); blades.lineTo(16, 16);
    blades.moveTo(7, 13.5); blades.lineTo(16, 4);
    painter.drawPath(blades);
}

// Toolbar symbols drawn on a shared grid so their weight and optical size
// match; an unavailable command's symbol is dimmed in both appearances, and
// a switch that is on (the conversation's column) has its symbol in the
// accent, as a chosen mode in a title strip does.
QIcon toolbarIcon(const QString &glyph) {
    const auto draw = [glyph](QPainter &painter, const QColor &color) {
        painter.setPen(QPen(color, 1.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        QPainterPath path;
        if (glyph == QStringLiteral("bubble")) {
            // SF Symbols' bubble.left: a rounded speech bubble, its tail at
            // the lower left, as the comments button beside a Short.
            path.moveTo(6.5, 3.5); path.lineTo(13.5, 3.5);
            path.quadTo(17, 3.5, 17, 7); path.lineTo(17, 10);
            path.quadTo(17, 13.5, 13.5, 13.5); path.lineTo(9.5, 13.5);
            path.lineTo(5.5, 16.5); path.lineTo(6.2, 13.5);
            path.quadTo(3, 13.3, 3, 10); path.lineTo(3, 7);
            path.quadTo(3, 3.5, 6.5, 3.5);
            path.closeSubpath();
        } else if (glyph == QStringLiteral("↓")) {
            path.moveTo(10, 3); path.lineTo(10, 12);
            path.moveTo(6.5, 8.5); path.lineTo(10, 12); path.lineTo(13.5, 8.5);
            path.moveTo(4, 12); path.lineTo(4, 16); path.lineTo(16, 16); path.lineTo(16, 12);
        } else if (glyph == QStringLiteral("↶") || glyph == QStringLiteral("↷")) {
            if (glyph == QStringLiteral("↷")) { painter.translate(20, 0); painter.scale(-1, 1); }
            path.moveTo(7, 4); path.lineTo(3.5, 7.5); path.lineTo(7, 11);
            path.moveTo(4, 7.5); path.lineTo(11, 7.5);
            path.cubicTo(17, 7.5, 17, 15.5, 11, 15.5);
        } else if (glyph == QStringLiteral("✂")) {
            drawScissors(painter);
        } else if (glyph == QStringLiteral("trash")) {
            // SF Symbols' trash: a lid with its handle over a can.
            path.moveTo(4, 5); path.lineTo(16, 5);
            path.moveTo(8, 5); path.lineTo(8, 3); path.lineTo(12, 3); path.lineTo(12, 5);
            path.moveTo(5.5, 6); path.lineTo(5.5, 14.5);
            path.quadTo(5.5, 16, 7, 16); path.lineTo(13, 16);
            path.quadTo(14.5, 16, 14.5, 14.5); path.lineTo(14.5, 6);
            path.moveTo(8.5, 8.5); path.lineTo(8.5, 13.5);
            path.moveTo(11.5, 8.5); path.lineTo(11.5, 13.5);
        } else {
            path.moveTo(5, 5); path.lineTo(15, 15);
            path.moveTo(15, 5); path.lineTo(5, 15);
        }
        painter.drawPath(path);
    };
    // Drawn at the size the toolbar shows them (kToolbarIconSize), the
    // 20-point grid scaled down as vectors: a 20-point picture shrunk to 15
    // by the icon engine blurred every stroke.
    const auto shown = [draw](QPainter &painter, const QColor &color) {
        painter.scale(kToolbarIconSize / 20.0, kToolbarIconSize / 20.0);
        draw(painter, color);
    };
    const QSize size(kToolbarIconSize, kToolbarIconSize);
    QIcon icon = glyphIcon(size, shown);
    const QPixmap chosen = glyphPixmap(size, shown, editorTheme().accentText);
    icon.addPixmap(chosen, QIcon::Normal, QIcon::On);
    icon.addPixmap(chosen, QIcon::Active, QIcon::On);
    return icon;
}

// A tool palette symbol on an 18-point grid, in `color`: the pointer for
// Selection (SF Symbols' cursorarrow), the edit suites' marks for the rest,
// drawn with the toolbar's pen so they share its weight.
static QPixmap toolGlyph(const QString &id, const QColor &color) {
    QPixmap pixmap(36, 36);
    pixmap.setDevicePixelRatio(2);
    pixmap.fill(Qt::transparent);
    QPainter painter(&pixmap);
    painter.setRenderHint(QPainter::Antialiasing);
    if (id == QStringLiteral("tool.selection")) {
        const QPolygonF arrow{{5.5, 2.0}, {5.5, 15.0}, {8.6, 12.0}, {10.7, 16.6}, {12.7, 15.7}, {10.6, 11.2}, {14.8, 11.2}};
        painter.setPen(QPen(color, 1.0, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        painter.setBrush(color);
        painter.drawPolygon(arrow);
        return pixmap;
    }
    painter.setPen(QPen(color, 1.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    if (id == QStringLiteral("tool.razor")) {
        // The toolbar's scissors, brought onto this grid.
        painter.scale(0.9, 0.9);
        drawScissors(painter);
        return pixmap;
    }
    // An arrow along y from x0 to x1, its head at x1.
    const auto arrow = [](QPainterPath &path, qreal y, qreal x0, qreal x1) {
        const qreal back = x1 > x0 ? -2.5 : 2.5;
        path.moveTo(x0, y); path.lineTo(x1, y);
        path.moveTo(x1 + back, y - 2.5); path.lineTo(x1, y); path.lineTo(x1 + back, y + 2.5);
    };
    QPainterPath path;
    if (id == QStringLiteral("tool.ripple")) {
        // An edit point, and the cut closing up on it.
        path.moveTo(4, 3.5); path.lineTo(4, 14.5);
        arrow(path, 9, 15, 7);
    } else if (id == QStringLiteral("tool.roll")) {
        // An edit point, moving either way.
        path.moveTo(9, 3.5); path.lineTo(9, 14.5);
        path.moveTo(5.5, 6.5); path.lineTo(3, 9); path.lineTo(5.5, 11.5);
        path.moveTo(12.5, 6.5); path.lineTo(15, 9); path.lineTo(12.5, 11.5);
    } else if (id == QStringLiteral("tool.slip")) {
        // The clip's content, slid either way inside it.
        arrow(path, 9, 9, 3);
        arrow(path, 9, 9, 15);
    } else if (id == QStringLiteral("tool.slide")) {
        // The clip, and its neighbours giving way.
        arrow(path, 6, 3.5, 14.5);
        arrow(path, 12, 14.5, 3.5);
    }
    painter.drawPath(path);
    return pixmap;
}

// A tool's icon: secondary text when not chosen, the accent when chosen.
QIcon toolIcon(const QString &id) {
    const EditorTheme &theme = editorTheme();
    QIcon icon;
    icon.addPixmap(toolGlyph(id, theme.textSecondary), QIcon::Normal, QIcon::Off);
    icon.addPixmap(toolGlyph(id, theme.textStrong), QIcon::Active, QIcon::Off);
    icon.addPixmap(toolGlyph(id, theme.accent), QIcon::Normal, QIcon::On);
    icon.addPixmap(toolGlyph(id, theme.accent), QIcon::Active, QIcon::On);
    icon.addPixmap(toolGlyph(id, theme.glyphDisabled), QIcon::Disabled, QIcon::Off);
    return icon;
}


} // namespace kerf::window_parts
