#include "browser_panel.h"
#include "components.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "effects_panel.h"
#include "presets_panel.h"
#include "tray_shell.h"

#include <QAbstractButton>
#include <QHBoxLayout>
#include <QLabel>
#include <QPainter>
#include <QAction>
#include <QStackedWidget>
#include <QVBoxLayout>

namespace kerf {

BrowserPanel::BrowserPanel(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("browserPanel"));
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    // The strip: 28 tall, pad 0 8 0 12.
    auto *strip = new QWidget;
    strip->setObjectName(QStringLiteral("browserStrip"));
    strip->setFixedHeight(tokens::layout::strip);
    auto *stripLayout = new QHBoxLayout(strip);
    m_stripLayout = stripLayout;
    stripLayout->setContentsMargins(tokens::space::s12, 0, tokens::space::s8, 0);
    stripLayout->setSpacing(tokens::space::s4);
    m_pages = new ui::SegmentedControl;
    m_pages->setObjectName(QStringLiteral("browserPages"));
    for (const char *label : {"Effects", "Titles", "Presets"}) m_pages->addSegment(QString::fromLatin1(label));
    for (int index = 0; index < 3; ++index) m_pages->segment(index)->setObjectName(QStringLiteral("browserPage_%1").arg(pageId(Page(index))));
    m_pages->setCurrentIndex(0, false);
    connect(m_pages, &ui::SegmentedControl::activated, this, [this](int index) { setPage(Page(index)); });
    stripLayout->addWidget(m_pages);
    stripLayout->addStretch(1);
    m_search = new ui::IconButton(QStringLiteral("g-search"));
    m_search->setObjectName(QStringLiteral("browserSearch"));
    connect(m_search, &QAbstractButton::clicked, this, &BrowserPanel::focusSearch);
    stripLayout->addWidget(m_search);
    layout->addWidget(strip);
    // ⌘⇧F is the registry's browser.search (setSearchAction).
    updateSearchTip();

    m_stack = new QStackedWidget;
    m_stack->setObjectName(QStringLiteral("browserPagesStack"));
    // Until the window gives a page its panel: no engine, nothing to browse.
    auto *placeholder = shell::text(QStringLiteral("Waiting for the Kerf engine"), tokens::meta, tokens::ink3);
    placeholder->setObjectName(QStringLiteral("browserTitlesEmpty"));
    placeholder->setAlignment(Qt::AlignCenter);
    m_placeholder = placeholder;
    m_stack->addWidget(m_placeholder);
    layout->addWidget(m_stack, 1);
}

void BrowserPanel::addToStrip(QWidget *widget) { m_stripLayout->addWidget(widget); }

void BrowserPanel::setEffects(EffectsPanel *effects) {
    if (m_effects == effects) return;
    if (m_effects) m_stack->removeWidget(m_effects);
    m_effects = effects;
    if (effects) m_stack->addWidget(effects);
    showCurrent();
}

void BrowserPanel::setTitles(EffectsPanel *titles) {
    if (m_titles == titles) return;
    if (m_titles) m_stack->removeWidget(m_titles);
    m_titles = titles;
    if (titles) m_stack->addWidget(titles);
    showCurrent();
}

void BrowserPanel::setPresets(PresetsPanel *presets) {
    if (m_presets == presets) return;
    if (m_presets) m_stack->removeWidget(m_presets);
    m_presets = presets;
    if (presets) m_stack->addWidget(presets);
    showCurrent();
}

void BrowserPanel::setPage(Page page) {
    if (!pageAvailable(page)) {
        m_pages->setCurrentIndex(int(m_page), false);
        return;
    }
    m_pages->setCurrentIndex(int(page));
    if (page == m_page) return;
    m_page = page;
    updateSearchTip();
    showCurrent();
    emit pageChanged(page);
}

bool BrowserPanel::pageAvailable(Page) {
    return true;
}

QString BrowserPanel::pageId(Page page) {
    switch (page) {
    case Page::Titles: return QStringLiteral("titles");
    case Page::Presets: return QStringLiteral("presets");
    case Page::Effects: break;
    }
    return QStringLiteral("effects");
}

BrowserPanel::Page BrowserPanel::pageFromId(const QString &id) {
    if (id == QStringLiteral("titles")) return Page::Titles;
    if (id == QStringLiteral("presets")) return Page::Presets;
    return Page::Effects;
}

QAbstractButton *BrowserPanel::pageButton(Page page) const {
    return qobject_cast<QAbstractButton *>(m_pages->segment(int(page)));
}

QAbstractButton *BrowserPanel::searchButton() const {
    return m_search;
}

QWidget *BrowserPanel::currentPage() const {
    return m_stack->currentWidget();
}

void BrowserPanel::focusSearch() {
    if (m_page == Page::Presets) setPage(Page::Effects);
    EffectsPanel *panel = m_page == Page::Titles ? m_titles : m_effects;
    if (panel) panel->focusSearch();
}

void BrowserPanel::setSearchAction(QAction *action) {
    if (m_searchAction) {
        removeAction(m_searchAction);
        m_searchAction->disconnect(this);
    }
    m_searchAction = action;
    if (action) {
        action->setShortcutContext(Qt::WindowShortcut);
        addAction(action);
        connect(action, &QAction::changed, this, &BrowserPanel::updateSearchTip);
    }
    updateSearchTip();
}

void BrowserPanel::updateSearchTip() {
    m_search->setTip(m_page == Page::Titles ? QStringLiteral("Search titles") : QStringLiteral("Search effects"),
                     m_searchAction ? m_searchAction->shortcut().toString(QKeySequence::NativeText) : QString());
}

void BrowserPanel::showCurrent() {
    QWidget *shown = m_page == Page::Presets ? static_cast<QWidget *>(m_presets)
                     : m_page == Page::Titles ? static_cast<QWidget *>(m_titles)
                                              : static_cast<QWidget *>(m_effects);
    m_stack->setCurrentWidget(shown ? shown : m_placeholder);
}

void BrowserPanel::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), tokens::color(tokens::chrome));
    // The strip's top and bottom lines (mockup #brw>.strip).
    painter.fillRect(QRect(0, 0, width(), 1), tokens::color(tokens::line));
    painter.fillRect(QRect(0, tokens::layout::strip - 1, width(), 1), tokens::color(tokens::line));
}

} // namespace kerf
