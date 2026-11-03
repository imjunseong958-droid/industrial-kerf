#include "browser_panel.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "effects_panel.h"
#include "presets_panel.h"

#include <QButtonGroup>
#include <QFrame>
#include <QHBoxLayout>
#include <QLabel>
#include <QPainter>
#include <QStackedWidget>
#include <QToolButton>
#include <QVBoxLayout>

namespace kerf {

namespace {

QString css(QRgb color) {
    const QColor c = tokens::color(color);
    return QStringLiteral("rgba(%1,%2,%3,%4)").arg(c.red()).arg(c.green()).arg(c.blue()).arg(c.alphaF(), 0, 'f', 3);
}

} // namespace

BrowserPanel::BrowserPanel(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("browserPanel"));
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    auto *strip = new QWidget;
    strip->setObjectName(QStringLiteral("browserStrip"));
    strip->setFixedHeight(tokens::layout::strip);
    auto *stripLayout = new QHBoxLayout(strip);
    stripLayout->setContentsMargins(tokens::space::s12, 0, tokens::space::s8, 0);
    stripLayout->setSpacing(tokens::space::s4);
    auto *track = new QFrame;
    track->setObjectName(QStringLiteral("browserPages"));
    track->setFixedHeight(tokens::height::h24);
    track->setStyleSheet(QStringLiteral(
        "QFrame#browserPages { background: %1; border-radius: 6px; }"
        "QToolButton { color: %2; background: transparent; border: none; border-radius: 4px; padding: 0 8px; }"
        "QToolButton:hover { color: %3; }"
        "QToolButton:checked { color: %3; background: %4; }"
        "QToolButton:disabled { color: %5; }")
                             .arg(css(tokens::raised), css(tokens::ink2), css(tokens::ink), css(tokens::on), css(tokens::dis)));
    auto *trackLayout = new QHBoxLayout(track);
    trackLayout->setContentsMargins(2, 2, 2, 2);
    trackLayout->setSpacing(0);
    auto *group = new QButtonGroup(this);
    const char *labels[] = {"Effects", "Titles", "Presets"};
    for (int index = 0; index < 3; ++index) {
        auto *button = new QToolButton;
        button->setText(QString::fromLatin1(labels[index]));
        button->setObjectName(QStringLiteral("browserPage_%1").arg(pageId(Page(index))));
        button->setCheckable(true);
        button->setFixedHeight(20);
        button->setFont(tokens::font(tokens::controlStrong));
        button->setCursor(Qt::PointingHandCursor);
        button->setEnabled(pageAvailable(Page(index)));
        if (!button->isEnabled()) button->setToolTip(QStringLiteral("%1\nComing in this release").arg(button->text()));
        group->addButton(button, index);
        trackLayout->addWidget(button);
        m_pageButtons[index] = button;
    }
    m_pageButtons[0]->setChecked(true);
    connect(group, &QButtonGroup::idClicked, this, [this](int id) { setPage(Page(id)); });
    stripLayout->addWidget(track);
    stripLayout->addStretch(1);
    m_search = new QToolButton;
    m_search->setObjectName(QStringLiteral("browserSearch"));
    m_search->setText(QStringLiteral("Search"));
    m_search->setToolTip(QStringLiteral("Search effects"));
    m_search->setAutoRaise(true);
    m_search->setFont(tokens::font(tokens::control));
    m_search->setFixedHeight(tokens::height::h24);
    connect(m_search, &QToolButton::clicked, this, [this] {
        setPage(Page::Effects);
        if (m_effects) m_effects->focusSearch();
    });
    stripLayout->addWidget(m_search);
    layout->addWidget(strip);
    m_stack = new QStackedWidget;
    m_stack->setObjectName(QStringLiteral("browserPagesStack"));
    auto *titles = new QLabel(QStringLiteral("Titles · coming in this release"));
    titles->setObjectName(QStringLiteral("browserTitlesEmpty"));
    titles->setAlignment(Qt::AlignCenter);
    m_titles = titles;
    m_stack->addWidget(m_titles);
    layout->addWidget(m_stack, 1);
}

void BrowserPanel::setEffects(EffectsPanel *effects) {
    if (m_effects == effects) return;
    if (m_effects) m_stack->removeWidget(m_effects);
    m_effects = effects;
    if (effects) m_stack->addWidget(effects);
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
        m_pageButtons[int(m_page)]->setChecked(true);
        return;
    }
    m_pageButtons[int(page)]->setChecked(true);
    if (page == m_page) return;
    m_page = page;
    showCurrent();
    emit pageChanged(page);
}

bool BrowserPanel::pageAvailable(Page page) {
    return page != Page::Titles;
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
    return m_pageButtons[int(page)];
}

QWidget *BrowserPanel::currentPage() const {
    return m_stack->currentWidget();
}

void BrowserPanel::showCurrent() {
    QWidget *shown = m_page == Page::Presets ? static_cast<QWidget *>(m_presets)
                     : m_page == Page::Titles ? m_titles
                                              : static_cast<QWidget *>(m_effects);
    m_stack->setCurrentWidget(shown ? shown : m_titles);
}

void BrowserPanel::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), tokens::color(tokens::chrome));
    painter.fillRect(QRect(0, tokens::layout::strip - 1, width(), 1), tokens::color(tokens::line));
}

} // namespace kerf
