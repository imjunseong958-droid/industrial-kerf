#include "inspector_column.h"
#include "components.h"
#include "deliver_pane.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "inspector_panel.h"
#include "source_monitor_panel.h"

#include <QAbstractButton>
#include <QHBoxLayout>
#include <QLabel>
#include <QPainter>
#include <QStackedWidget>
#include <QVBoxLayout>

namespace kerf {

namespace {

// The column's pages. The column keeps its width (304 at the design's
// window) whichever page shows: its least size is the Inspector's and the
// Deliver pane's, not the Source monitor's.
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

InspectorPanel::Pane paneOf(InspectorColumn::Kind kind) {
    switch (kind) {
    case InspectorColumn::Kind::Audio: return InspectorPanel::Pane::Audio;
    case InspectorColumn::Kind::Info: return InspectorPanel::Pane::Info;
    case InspectorColumn::Kind::Video: break;
    }
    return InspectorPanel::Pane::Video;
}

} // namespace

InspectorColumn::InspectorColumn(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("inspectorColumn"));
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    // The strip (spec 5): 28, pad 0 12, a line at its foot; the segmented
    // Inspector | Source, then the w20 icon segments Video, Audio, Info.
    auto *strip = new QWidget;
    strip->setObjectName(QStringLiteral("inspectorStrip"));
    strip->setFixedHeight(tokens::layout::strip);
    auto *stripLayout = new QHBoxLayout(strip);
    m_stripLayout = stripLayout;
    stripLayout->setContentsMargins(tokens::space::s12, 0, tokens::space::s8, 0);
    stripLayout->setSpacing(tokens::space::s4);
    m_pages = new ui::SegmentedControl;
    m_pages->setObjectName(QStringLiteral("inspectorPages"));
    m_pages->addSegment(QStringLiteral("Inspector"), QStringLiteral("Inspector"));
    m_pages->addSegment(QStringLiteral("Source"), QStringLiteral("Source"));
    m_pages->setCurrentIndex(0, false);
    stripLayout->addWidget(m_pages);
    stripLayout->addStretch(1);
    m_kinds = new ui::SegmentedControl;
    m_kinds->setObjectName(QStringLiteral("inspectorKinds"));
    m_kinds->setIconSegmentWidth(20);
    m_kinds->addIconSegment(QStringLiteral("g-video"), QStringLiteral("Video"));
    m_kinds->addIconSegment(QStringLiteral("g-audio"), QStringLiteral("Audio"));
    m_kinds->addIconSegment(QStringLiteral("g-info"), QStringLiteral("Info"));
    m_kinds->setCurrentIndex(0, false);
    const char *names[] = {"Video", "Audio", "Info"};
    for (int index = 0; index < 3; ++index) {
        m_kindButtons[index] = qobject_cast<QAbstractButton *>(m_kinds->segment(index));
        if (m_kindButtons[index]) m_kindButtons[index]->setAccessibleName(QString::fromLatin1(names[index]));
    }
    for (int index = 0; index < 2; ++index) m_pageButtons[index] = qobject_cast<QAbstractButton *>(m_pages->segment(index));
    stripLayout->addWidget(m_kinds);
    connect(m_pages, &ui::SegmentedControl::activated, this, [this](int index) { setPage(Page(index)); });
    connect(m_kinds, &ui::SegmentedControl::activated, this, [this](int index) {
        // A kind chosen shows its pane on the Inspector page (as the mockup's
        // kind segments turn the Inspector tab back on).
        setKind(Kind(index));
        setPage(Page::Inspector);
    });
    layout->addWidget(strip);
    m_stack = new PageStack;
    m_stack->setObjectName(QStringLiteral("inspectorPagesStack"));
    auto *empty = new QLabel(QStringLiteral("Nothing to inspect"));
    empty->setAlignment(Qt::AlignCenter);
    empty->setFont(tokens::font(tokens::meta));
    empty->setStyleSheet(QStringLiteral("QLabel { color: %1; }").arg(tokens::color(tokens::ink3).name()));
    m_empty = empty;
    m_stack->addWidget(m_empty);
    m_deliver = new DeliverPane(nullptr, bus);
    m_stack->addWidget(m_deliver);
    layout->addWidget(m_stack, 1);
    if (m_bus)
        // A tray opening or closing goes back to the Inspector (spec 10) and
        // shows the Deliver pane in Deliver.
        connect(m_bus, &EditorBus::workspaceChanged, this, [this] {
            setPage(Page::Inspector);
            showCurrent();
        });
    showCurrent();
}

void InspectorColumn::addToStrip(QWidget *widget) { m_stripLayout->addWidget(widget); }

void InspectorColumn::setInspector(InspectorPanel *inspector) {
    if (m_inspector == inspector) return;
    if (m_inspector) m_stack->removeWidget(m_inspector);
    m_inspector = inspector;
    if (inspector) {
        m_stack->addWidget(inspector);
        inspector->setPane(paneOf(m_kind));
    }
    showCurrent();
}

void InspectorColumn::setSourceMonitor(SourceMonitorPanel *source) {
    if (m_source == source) return;
    if (m_source) m_stack->removeWidget(m_source);
    m_source = source;
    static_cast<PageStack *>(m_stack)->loose = source;
    if (source) {
        // Narrower than its least, it is cut at the column's edge until
        // Q-SOURCE gives it its compact form.
        source->setMinimumWidth(0);
        m_stack->addWidget(source);
    }
    showCurrent();
}

void InspectorColumn::setPage(Page page) {
    m_pages->setCurrentIndex(int(page));
    if (page == m_page) return;
    m_page = page;
    showCurrent();
    emit pageChanged(page);
}

void InspectorColumn::setKind(Kind kind) {
    if (!kindAvailable(kind)) {
        m_kinds->setCurrentIndex(int(m_kind));
        return;
    }
    m_kinds->setCurrentIndex(int(kind));
    if (kind == m_kind) return;
    m_kind = kind;
    if (m_inspector) m_inspector->setPane(paneOf(kind));
    showCurrent();
    emit kindChanged(kind);
}

bool InspectorColumn::kindAvailable(Kind) {
    // Video, Audio and Info each have their pane (Q-INSPECTOR, W2).
    return true;
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
           m_bus->workspace() == QLatin1String("Deliver");
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

void InspectorColumn::setKeys(const QString &inspector, const QString &video, const QString &audio, const QString &info) {
    m_pages->setSegmentTip(0, QStringLiteral("Inspector"), inspector);
    m_kinds->setSegmentTip(0, QStringLiteral("Video"), video);
    m_kinds->setSegmentTip(1, QStringLiteral("Audio"), audio);
    m_kinds->setSegmentTip(2, QStringLiteral("Info"), info);
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
