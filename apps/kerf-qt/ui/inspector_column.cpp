#include "inspector_column.h"
#include "deliver_pane.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "inspector_panel.h"
#include "source_monitor_panel.h"
#include "workspace.h"

#include <QButtonGroup>
#include <QFrame>
#include <QHBoxLayout>
#include <QLabel>
#include <QIcon>
#include <QPainter>
#include <QPainterPath>
#include <QPixmap>
#include <QStackedWidget>
#include <QToolButton>
#include <QVBoxLayout>

namespace kerf {

namespace {

QString css(QRgb color) {
    const QColor c = tokens::color(color);
    return QStringLiteral("rgba(%1,%2,%3,%4)").arg(c.red()).arg(c.green()).arg(c.blue()).arg(c.alphaF(), 0, 'f', 3);
}

// A segmented control's track, its segments text buttons that latch one at a time.
QFrame *segmented(const QString &name, QButtonGroup *group, const QStringList &labels, QList<QToolButton *> &buttons) {
    auto *track = new QFrame;
    track->setObjectName(name);
    track->setFixedHeight(tokens::height::h24);
    track->setStyleSheet(QStringLiteral(
        "QFrame#%1 { background: %2; border-radius: 6px; }"
        "QToolButton { color: %3; background: transparent; border: none; border-radius: 4px; padding: 0 8px; }"
        "QToolButton:hover { color: %4; }"
        "QToolButton:checked { color: %4; background: %5; }"
        "QToolButton:disabled { color: %6; }")
                             .arg(name, css(tokens::raised), css(tokens::ink2), css(tokens::ink), css(tokens::on), css(tokens::dis)));
    auto *layout = new QHBoxLayout(track);
    layout->setContentsMargins(2, 2, 2, 2);
    layout->setSpacing(0);
    for (const QString &label : labels) {
        auto *button = new QToolButton;
        button->setText(label);
        button->setCheckable(true);
        button->setFixedHeight(20);
        button->setFont(tokens::font(tokens::controlStrong));
        button->setCursor(Qt::PointingHandCursor);
        group->addButton(button, int(buttons.size()));
        layout->addWidget(button);
        buttons.append(button);
    }
    return track;
}

// The column's pages. The column keeps its width (304 at the design's
// window) whichever page shows: its least size is the Inspector's and the
// Deliver pane's, not the Source monitor's, which Q-SOURCE makes compact
// for the column in W2.
class PageStack final : public QStackedWidget {
public:
    QWidget *loose = nullptr;
    QSize minimumSizeHint() const override {
        QSize least(0, 0);
        for (int index = 0; index < count(); ++index)
            if (widget(index) != loose) least = least.expandedTo(widget(index)->minimumSizeHint());
        return least;
    }
};

// A kind's symbol on a 14-point grid, in each state's ink.
QIcon kindIcon(InspectorColumn::Kind kind) {
    QIcon icon;
    const struct {
        QIcon::Mode mode;
        QIcon::State state;
        QRgb ink;
    } inks[] = {{QIcon::Normal, QIcon::Off, tokens::ink2}, {QIcon::Normal, QIcon::On, tokens::ink},
                {QIcon::Active, QIcon::Off, tokens::ink}, {QIcon::Active, QIcon::On, tokens::ink},
                {QIcon::Disabled, QIcon::Off, tokens::dis}, {QIcon::Disabled, QIcon::On, tokens::dis}};
    for (const auto &entry : inks) {
        QPixmap pixmap(28, 28);
        pixmap.setDevicePixelRatio(2);
        pixmap.fill(Qt::transparent);
        QPainter painter(&pixmap);
        painter.setRenderHint(QPainter::Antialiasing);
        painter.setPen(QPen(tokens::color(entry.ink), 1.3, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        switch (kind) {
        case InspectorColumn::Kind::Video:
            painter.drawRoundedRect(QRectF(1.5, 3.5, 11, 7), 1.5, 1.5);
            break;
        case InspectorColumn::Kind::Audio: {
            QPainterPath speaker;
            speaker.moveTo(2, 5.5); speaker.lineTo(4.5, 5.5); speaker.lineTo(7.5, 3); speaker.lineTo(7.5, 11);
            speaker.lineTo(4.5, 8.5); speaker.lineTo(2, 8.5); speaker.closeSubpath();
            painter.drawPath(speaker);
            painter.drawArc(QRectF(7, 4, 5, 6), -60 * 16, 120 * 16);
            break;
        }
        case InspectorColumn::Kind::Info:
            painter.drawEllipse(QRectF(1.5, 1.5, 11, 11));
            painter.drawLine(QPointF(7, 6.2), QPointF(7, 10));
            painter.drawPoint(QPointF(7, 4.2));
            break;
        }
        painter.end();
        icon.addPixmap(pixmap, entry.mode, entry.state);
    }
    return icon;
}

} // namespace

InspectorColumn::InspectorColumn(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("inspectorColumn"));
    auto *layout = new QVBoxLayout(this);
    // The column's left seam is the dock splitter's line.
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    auto *strip = new QWidget;
    strip->setObjectName(QStringLiteral("inspectorStrip"));
    strip->setFixedHeight(tokens::layout::strip);
    auto *stripLayout = new QHBoxLayout(strip);
    stripLayout->setContentsMargins(tokens::space::s12, 0, tokens::space::s12, 0);
    stripLayout->setSpacing(tokens::space::s4);
    auto *pages = new QButtonGroup(this);
    QList<QToolButton *> pageButtons;
    stripLayout->addWidget(segmented(QStringLiteral("inspectorPages"), pages, {QStringLiteral("Inspector"), QStringLiteral("Source")}, pageButtons));
    stripLayout->addStretch(1);
    auto *kinds = new QButtonGroup(this);
    QList<QToolButton *> kindButtons;
    stripLayout->addWidget(segmented(QStringLiteral("inspectorKinds"), kinds, {QString(), QString(), QString()}, kindButtons));
    // The kinds are 20-point icon segments (Q-INSPECTOR draws the kit's
    // icons for them in W2): a frame, a speaker, an "i".
    for (int index = 0; index < 3; ++index) {
        QToolButton *button = kindButtons.at(index);
        button->setFixedSize(20, 20);
        button->setIconSize(QSize(14, 14));
        button->setAccessibleName(QStringList{QStringLiteral("Video"), QStringLiteral("Audio"), QStringLiteral("Info")}.at(index));
        button->setIcon(kindIcon(Kind(index)));
    }
    for (int index = 0; index < 2; ++index) m_pageButtons[index] = pageButtons.at(index);
    const char *kindTips[] = {"Video (⌃⌥4)", "Audio (⌃⌥5)", "Info (⌃⌥6)"};
    for (int index = 0; index < 3; ++index) {
        m_kindButtons[index] = kindButtons.at(index);
        const bool available = kindAvailable(Kind(index));
        m_kindButtons[index]->setEnabled(available);
        m_kindButtons[index]->setToolTip(available ? QString::fromUtf8(kindTips[index])
                                                   : QStringLiteral("%1\nComing in this release").arg(QString::fromUtf8(kindTips[index])));
    }
    m_pageButtons[0]->setChecked(true);
    m_kindButtons[0]->setChecked(true);
    connect(pages, &QButtonGroup::idClicked, this, [this](int id) { setPage(Page(id)); });
    connect(kinds, &QButtonGroup::idClicked, this, [this](int id) { setKind(Kind(id)); });
    layout->addWidget(strip);
    m_stack = new PageStack;
    m_stack->setObjectName(QStringLiteral("inspectorPagesStack"));
    auto *empty = new QLabel(QStringLiteral("Nothing to inspect"));
    empty->setAlignment(Qt::AlignCenter);
    m_empty = empty;
    m_stack->addWidget(m_empty);
    m_deliver = new DeliverPane(nullptr, bus);
    m_stack->addWidget(m_deliver);
    layout->addWidget(m_stack, 1);
    if (m_bus)
        // A workspace switch goes back to the Inspector (spec 10) and shows
        // the Deliver pane in Deliver.
        connect(m_bus, &EditorBus::workspaceChanged, this, [this] {
            setPage(Page::Inspector);
            showCurrent();
        });
    showCurrent();
}

void InspectorColumn::setInspector(InspectorPanel *inspector) {
    if (m_inspector == inspector) return;
    if (m_inspector) m_stack->removeWidget(m_inspector);
    m_inspector = inspector;
    if (inspector) m_stack->addWidget(inspector);
    showCurrent();
}

void InspectorColumn::setSourceMonitor(SourceMonitorPanel *source) {
    if (m_source == source) return;
    if (m_source) m_stack->removeWidget(m_source);
    m_source = source;
    static_cast<PageStack *>(m_stack)->loose = source;
    if (source) {
        // Narrower than its least, it is cut at the column's edge until then.
        source->setMinimumWidth(0);
        m_stack->addWidget(source);
    }
    showCurrent();
}

void InspectorColumn::setPage(Page page) {
    m_pageButtons[int(page)]->setChecked(true);
    if (page == m_page) return;
    m_page = page;
    showCurrent();
    emit pageChanged(page);
}

void InspectorColumn::setKind(Kind kind) {
    if (!kindAvailable(kind)) {
        m_kindButtons[int(m_kind)]->setChecked(true);
        return;
    }
    m_kindButtons[int(kind)]->setChecked(true);
    if (kind == m_kind) return;
    m_kind = kind;
    showCurrent();
    emit kindChanged(kind);
}

bool InspectorColumn::kindAvailable(Kind kind) {
    return kind == Kind::Video;
}

QString InspectorColumn::kindId(Kind kind) {
    switch (kind) {
    case Kind::Audio: return QStringLiteral("audio");
    case Kind::Info: return QStringLiteral("info");
    case Kind::Video: break;
    }
    return QStringLiteral("video");
}

InspectorColumn::Kind InspectorColumn::kindFromId(const QString &id) {
    if (id == QStringLiteral("audio")) return Kind::Audio;
    if (id == QStringLiteral("info")) return Kind::Info;
    return Kind::Video;
}

bool InspectorColumn::deliverShown() const {
    return m_page == Page::Inspector && m_kind == Kind::Video && m_bus &&
           m_bus->workspace() == QLatin1String(Workspace::kDeliver);
}

QAbstractButton *InspectorColumn::pageButton(Page page) const {
    return m_pageButtons[int(page)];
}

QAbstractButton *InspectorColumn::kindButton(Kind kind) const {
    return m_kindButtons[int(kind)];
}

QWidget *InspectorColumn::currentPage() const {
    return m_stack->currentWidget();
}

void InspectorColumn::showCurrent() {
    QWidget *shown = m_page == Page::Source ? static_cast<QWidget *>(m_source)
                     : deliverShown()       ? static_cast<QWidget *>(m_deliver)
                                            : static_cast<QWidget *>(m_inspector);
    m_stack->setCurrentWidget(shown ? shown : m_empty);
}

void InspectorColumn::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), tokens::color(tokens::chrome));
    painter.fillRect(QRect(0, tokens::layout::strip - 1, width(), 1), tokens::color(tokens::line));
}

} // namespace kerf
